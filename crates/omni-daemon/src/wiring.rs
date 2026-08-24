//! One client connection, and the sessions it is listening to.
//!
//! Everything a connection is written to goes through a queue and a writer
//! thread. A conductor delivering an event must never wait on a socket: a
//! client that has stopped reading is that client's problem, not the session's.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use omni_core::chat::Snapshot;
use omni_core::events::Event;
use omni_core::wire::{Frame, Reply};

pub struct Connection {
    pub id: u64,
    outbox: Sender<String>,
    open: Arc<AtomicBool>,
    /// Sessions this connection has opened, so detaching is tidy.
    pub listening: Mutex<Vec<String>>,
}

impl Connection {
    pub fn new(id: u64, stream: UnixStream) -> Arc<Self> {
        let (outbox, lines) = channel::<String>();
        let open = Arc::new(AtomicBool::new(true));
        let closed = open.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            for line in lines {
                if stream.write_all(line.as_bytes()).is_err() || stream.flush().is_err() {
                    break;
                }
            }
            closed.store(false, Ordering::SeqCst);
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });
        Arc::new(Connection {
            id,
            outbox,
            open,
            listening: Mutex::new(Vec::new()),
        })
    }

    pub fn open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn write(&self, line: String) -> bool {
        self.open() && self.outbox.send(line + "\n").is_ok()
    }

    pub fn reply(&self, reply: Reply) {
        if let Ok(line) = serde_json::to_string(&reply) {
            self.write(line);
        }
    }

    pub fn frame(&self, frame: &Frame) -> bool {
        match serde_json::to_string(frame) {
            Ok(line) => self.write(line),
            Err(_) => false,
        }
    }

    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
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
        if event.seq < self.next.load(Ordering::SeqCst) {
            return true; // already told; not a failure
        }
        self.next.store(event.seq + 1, Ordering::SeqCst);
        self.conn
            .frame(&Frame::event(session, event.clone(), snapshot))
    }
}
