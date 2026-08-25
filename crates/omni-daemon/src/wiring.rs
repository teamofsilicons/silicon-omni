//! One client connection, and the sessions it is listening to.
//!
//! Everything a connection is written to goes through a queue and a writer
//! thread. A conductor delivering an event must never wait on a socket: a
//! client that has stopped reading is that client's problem, not the session's.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omni_core::chat::Snapshot;
use omni_core::events::Event;
use omni_core::wire::{Frame, Reply};

/// Enough room for a burst or a substantial replay, but finite: a client that
/// stops reading must lose its connection rather than consume daemon memory
/// forever. It can reconnect from the last sequence it handled.
const OUTBOX_CAPACITY: usize = 4096;

/// A replay may briefly outrun the socket writer even when the client is
/// actively reading. Give that writer a bounded chance to catch up without
/// ever turning a stalled client into an indefinitely stalled session open.
const REPLAY_OUTBOX_TIMEOUT: Duration = Duration::from_secs(5);
const REPLAY_OUTBOX_RETRY: Duration = Duration::from_millis(2);

enum Outgoing {
    Line(String),
    Flush(SyncSender<()>),
}

pub struct Connection {
    pub id: u64,
    outbox: SyncSender<Outgoing>,
    open: Arc<AtomicBool>,
    socket: Mutex<Option<UnixStream>>,
    /// Sessions this connection has opened, so detaching is tidy.
    pub listening: Mutex<Vec<String>>,
}

impl Connection {
    pub fn new(id: u64, stream: UnixStream) -> Arc<Self> {
        let control = stream.try_clone().ok();
        let (outbox, lines) = sync_channel::<Outgoing>(OUTBOX_CAPACITY);
        let open = Arc::new(AtomicBool::new(true));
        let closed = open.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            for outgoing in lines {
                match outgoing {
                    Outgoing::Line(line) => {
                        if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
                            break;
                        }
                    }
                    Outgoing::Flush(done) => {
                        if stream.flush().is_err() {
                            break;
                        }
                        let _ = done.send(());
                    }
                }
            }
            closed.store(false, Ordering::SeqCst);
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });
        Arc::new(Connection {
            id,
            outbox,
            open,
            socket: Mutex::new(control),
            listening: Mutex::new(Vec::new()),
        })
    }

    pub fn open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn write(&self, line: String) -> bool {
        if !self.open() {
            return false;
        }
        match self.outbox.try_send(Outgoing::Line(line + "\n")) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.close();
                false
            }
        }
    }

    fn write_replay(&self, line: String, timeout: Duration) -> bool {
        if !self.open() {
            return false;
        }

        let deadline = Instant::now().checked_add(timeout);
        let mut outgoing = Outgoing::Line(line + "\n");
        loop {
            if !self.open() {
                return false;
            }
            match self.outbox.try_send(outgoing) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => {
                    self.close();
                    return false;
                }
                Err(TrySendError::Full(returned)) => {
                    outgoing = returned;
                    let Some(remaining) =
                        deadline.and_then(|end| end.checked_duration_since(Instant::now()))
                    else {
                        self.close();
                        return false;
                    };
                    if remaining.is_zero() {
                        self.close();
                        return false;
                    }
                    std::thread::sleep(remaining.min(REPLAY_OUTBOX_RETRY));
                }
            }
        }
    }

    pub fn reply(&self, reply: Reply) -> bool {
        if let Ok(line) = serde_json::to_string(&reply) {
            return self.write(line);
        }
        false
    }

    pub fn frame(&self, frame: &Frame) -> bool {
        match serde_json::to_string(frame) {
            Ok(line) => self.write(line),
            Err(_) => false,
        }
    }

    /// Queue one historical frame, allowing the bounded socket writer to make
    /// room. Live delivery deliberately uses [`Self::frame`] instead: it must
    /// never make a conductor wait on one client's socket.
    pub fn replay_frame(&self, frame: &Frame) -> bool {
        match serde_json::to_string(frame) {
            Ok(line) => self.write_replay(line, REPLAY_OUTBOX_TIMEOUT),
            Err(_) => false,
        }
    }

    /// Wait until everything queued before this call reached the socket.
    /// Used for the shutdown reply: closing the connection immediately after
    /// enqueueing it would otherwise race the writer thread and lose the one
    /// acknowledgement a lifecycle client is waiting for.
    pub fn flush(&self, timeout: Duration) -> bool {
        if !self.open() {
            return false;
        }
        let (done, waited) = sync_channel(0);
        match self.outbox.try_send(Outgoing::Flush(done)) {
            Ok(()) => waited.recv_timeout(timeout).is_ok(),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.close();
                false
            }
        }
    }

    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        if let Some(socket) = self.socket.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// One connection's interest in one session.
///
/// `next` is the position it has been told up to, so a replay from disk and the
/// live stream can run at the same time without a gap or a repeat.
pub struct Listener {
    pub conn: Arc<Connection>,
    next: AtomicI64,
}

impl Listener {
    pub fn new(conn: Arc<Connection>, from: i64) -> Arc<Self> {
        Arc::new(Listener {
            conn,
            next: AtomicI64::new(from),
        })
    }

    /// Send this event on, unless this listener has already had it.
    pub fn deliver(&self, session: &str, event: &Event, snapshot: Snapshot) -> bool {
        // A negative sequence is an explicitly unpersisted terminal notice.
        // Storage may be the thing that failed, so this is the one event the
        // conductor must be able to report without appending it first. It does
        // not move the replay cursor: a later reopen can reuse the next real
        // sequence without making this listener skip it.
        if event.seq >= 0 {
            if event.seq < self.next.load(Ordering::SeqCst) {
                return true; // already told; not a failure
            }
            self.next.store(event.seq + 1, Ordering::SeqCst);
        }
        self.conn
            .frame(&Frame::event(session, event.clone(), snapshot))
    }

    /// Replay a persisted event with bounded backpressure. `Live::attach`
    /// serialises replay with subscription, so this remains ordered with the
    /// non-blocking live path in [`Self::deliver`].
    pub fn replay(&self, session: &str, event: &Event, snapshot: Snapshot) -> bool {
        if event.seq >= 0 {
            if event.seq < self.next.load(Ordering::SeqCst) {
                return true;
            }
            self.next.store(event.seq + 1, Ordering::SeqCst);
        }
        self.conn
            .replay_frame(&Frame::event(session, event.clone(), snapshot))
    }
}

#[cfg(test)]
mod tests {
    use omni_core::choose::Ask;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::Receiver;

    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            session: "s".into(),
            status: omni_core::chat::STOPPED.into(),
            ask: Ask::intelligence(5),
            providers: Vec::new(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: String::new(),
            seq: 3,
            in_turn: false,
            queued: 0,
        }
    }

    fn connection_with_idle_writer(
        capacity: usize,
    ) -> (Connection, Receiver<Outgoing>, UnixStream) {
        let (socket, peer) = UnixStream::pair().unwrap();
        let (outbox, receiver) = sync_channel(capacity);
        (
            Connection {
                id: 1,
                outbox,
                open: Arc::new(AtomicBool::new(true)),
                socket: Mutex::new(Some(socket)),
                listening: Mutex::new(Vec::new()),
            },
            receiver,
            peer,
        )
    }

    fn outgoing_line(outgoing: Outgoing) -> String {
        match outgoing {
            Outgoing::Line(line) => line,
            Outgoing::Flush(_) => panic!("expected an ordinary line"),
        }
    }

    #[test]
    fn an_unpersisted_terminal_notice_is_delivered_without_moving_the_cursor() {
        let (writer, reader) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, writer);
        let listener = Listener::new(conn, 4);
        let event = Event::failure("omni", "session persistence failed");

        assert!(listener.deliver("s", &event, snapshot()));
        assert_eq!(listener.next.load(Ordering::SeqCst), 4);

        let mut line = String::new();
        BufReader::new(reader).read_line(&mut line).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(frame["event"]["seq"], serde_json::Value::Null);
        assert_eq!(frame["event"]["kind"], "omni");
        assert_eq!(frame["snapshot"]["status"], omni_core::chat::STOPPED);
    }

    #[test]
    fn flushing_waits_for_every_earlier_line_to_reach_the_socket() {
        let (writer, reader) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, writer);
        assert!(conn.reply(Reply::ok(7, serde_json::json!({"done": true}))));
        assert!(conn.replay_frame(&Frame::gone("s", snapshot())));
        assert!(conn.flush(Duration::from_secs(1)));

        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(reply.id, 7);
        assert!(reply.ok);

        line.clear();
        reader.read_line(&mut line).unwrap();
        let frame: Frame = serde_json::from_str(&line).unwrap();
        assert_eq!(frame.stream, "gone");
        assert_eq!(frame.session, "s");
    }

    #[test]
    fn live_delivery_closes_immediately_when_its_outbox_is_full() {
        let (conn, _receiver, _peer) = connection_with_idle_writer(1);
        conn.outbox
            .try_send(Outgoing::Line("already queued\n".into()))
            .unwrap();

        let started = Instant::now();
        assert!(!conn.frame(&Frame::gone("s", snapshot())));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(!conn.open());
    }

    #[test]
    fn replay_waits_for_capacity_and_keeps_fifo_order() {
        let (conn, receiver, _peer) = connection_with_idle_writer(1);
        conn.outbox
            .try_send(Outgoing::Line("first\n".into()))
            .unwrap();
        let reader = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            let first = outgoing_line(receiver.recv().unwrap());
            let second = outgoing_line(receiver.recv().unwrap());
            (first, second)
        });

        let frame = Frame::gone("s", snapshot());
        assert!(conn.write_replay(
            serde_json::to_string(&frame).unwrap(),
            Duration::from_secs(1)
        ));
        assert!(conn.open());

        let (first, second) = reader.join().unwrap();
        assert_eq!(first, "first\n");
        let actual: Frame = serde_json::from_str(second.trim()).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(frame).unwrap()
        );
    }

    #[test]
    fn replay_times_out_and_closes_a_non_reading_connection() {
        let (conn, _receiver, _peer) = connection_with_idle_writer(1);
        conn.outbox
            .try_send(Outgoing::Line("already queued\n".into()))
            .unwrap();

        let timeout = Duration::from_millis(25);
        let started = Instant::now();
        assert!(!conn.write_replay("never queued".into(), timeout));
        assert!(started.elapsed() >= timeout);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!conn.open());
    }
}
