//! `omnid` — the persistent core.
//!
//! One daemon per `OMNI_HOME`, listening on a Unix socket. It owns every
//! session and every provider process; the Python package, the CLI and the Rust
//! client are all thin things that talk to it over the socket. Keeping the
//! providers up between turns and between clients is the whole point: a session
//! you come back to is already warm.
//!
//! Run it in the foreground. Clients start it detached when they cannot find
//! one, so there is nothing to configure and nothing to remember.

mod registry;
mod serve;
mod wiring;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use omni_core::shared::paths;
use omni_core::wire::{Reply, Request};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::{Handle as SignalHandle, Signals};

use serve::Daemon;
use wiring::Connection;

/// How often to look for sessions nobody wants any more.
const SWEEP: Duration = Duration::from_secs(30);
/// Large enough for a many-megabyte prompt, finite enough that one malformed
/// local client cannot make the persistent daemon allocate until it dies.
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
/// Requests are blocking work, so each owns an OS thread. Socket readers apply
/// backpressure once this many tasks are active across the whole daemon.
const MAX_IN_FLIGHT: usize = 128;
const SHUTTING_DOWN: &str = "the omni daemon is shutting down";

fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("omnid {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(why) = run() {
        eprintln!("omnid: {why}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let socket = paths::socket();
    paths::ensure(&paths::home())
        .map_err(|err| format!("cannot use {:?}: {err}", paths::home()))?;
    // Hold an OS lock for the daemon's entire lifetime. Checking only the
    // socket has a startup race: two clients can both decide a stale path is
    // removable, unlink each other's socket, and leave two cores alive.
    let _daemon_lock = lock_daemon(&paths::daemon_lock())?;
    claim(&socket)?;
    let listener =
        UnixListener::bind(&socket).map_err(|err| format!("cannot listen on {socket:?}: {err}"))?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("cannot secure daemon socket {socket:?}: {err}"))?;
    let daemon = Arc::new(Daemon::default());
    let admission = Arc::new(Admission::new(MAX_IN_FLIGHT));
    let (signal_handle, signal_thread) = match shutdown_signals(&daemon, &admission, &socket) {
        Ok(signals) => signals,
        Err(err) => {
            let _ = std::fs::remove_file(&socket);
            return Err(err);
        }
    };
    say(&format!("listening on {}", socket.display()));
    prime();
    sweep(daemon.clone());
    let counter = AtomicU64::new(1);

    for stream in listener.incoming() {
        if daemon.stopping.load(Ordering::SeqCst) {
            break;
        }
        match stream {
            Ok(stream) => {
                let id = counter.fetch_add(1, Ordering::SeqCst);
                let daemon = daemon.clone();
                let admission = admission.clone();
                std::thread::spawn(move || talk(daemon, admission, id, stream));
            }
            Err(err) => say(&format!("a connection went wrong: {err}")),
        }
    }
    signal_handle.close();
    let _ = signal_thread.join();
    admission.close_and_wait();
    daemon.stopping.store(true, Ordering::SeqCst);
    say("shutting down");
    daemon.registry.shutdown();
    omni_core::providers::login::shutdown();
    let _ = std::fs::remove_file(&socket);
    Ok(())
}

/// Translate process signals into the same wakeup and cleanup path used by a
/// protocol `shutdown` request. `signal-hook` keeps all allocation and socket
/// work out of the actual signal handler; this thread wakes on its self-pipe.
fn shutdown_signals(
    daemon: &Arc<Daemon>,
    admission: &Arc<Admission>,
    socket: &std::path::Path,
) -> Result<(SignalHandle, JoinHandle<()>), String> {
    let mut signals =
        Signals::new([SIGTERM, SIGINT]).map_err(|err| format!("cannot watch signals: {err}"))?;
    let handle = signals.handle();
    let daemon = Arc::downgrade(daemon);
    let admission = admission.clone();
    let socket = socket.to_path_buf();
    let thread = std::thread::spawn(move || {
        for _signal in signals.forever() {
            let Some(daemon) = daemon.upgrade() else {
                return;
            };
            // Work admitted before the signal gets its ordinary answer. New
            // work is rejected, and only then does the accept loop unwind.
            admission.close_and_wait();
            daemon.stopping.store(true, Ordering::SeqCst);
            // `incoming` is blocking, so make it accept one harmless client
            // and observe the stopping flag at the top of its loop.
            let _ = UnixStream::connect(&socket);
        }
    });
    Ok((handle, thread))
}

/// An advisory lock on a persistent inode. The file deliberately remains
/// after a clean exit: deleting a locked file creates an inode race in which a
/// new process can lock a replacement while an older process still owns the
/// unlinked original. A stale PID is harmless; the kernel lock is authoritative.
fn lock_daemon(path: &std::path::Path) -> Result<std::fs::File, String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| format!("cannot open daemon lock {path:?}: {err}"))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("cannot secure daemon lock {path:?}: {err}"))?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked != 0 {
        return Err(format!("another omnid already owns {}", path.display()));
    }
    file.set_len(0)
        .and_then(|_| file.write_all(std::process::id().to_string().as_bytes()))
        .map_err(|err| format!("cannot write daemon lock {path:?}: {err}"))?;
    Ok(file)
}

/// Make sure we are the only daemon on this home.
///
/// A socket file left by a daemon that is no longer there is not a conflict —
/// it is litter, and refusing to start because of it would mean a crash wedges
/// omni until somebody deletes a file they have never heard of.
fn claim(socket: &std::path::Path) -> Result<(), String> {
    if !socket.exists() {
        return Ok(());
    }
    if UnixStream::connect(socket).is_ok() {
        return Err(format!(
            "another omnid is already listening on {}",
            socket.display()
        ));
    }
    std::fs::remove_file(socket).map_err(|err| format!("cannot clear a stale socket: {err}"))
}

/// A FIFO lane for one session on one connection.
///
/// Requests for unrelated sessions may be slow independently, but `open`,
/// `send`, and `set` for the same session must retain the order in which the
/// client wrote them. Tickets give us both properties without making the
/// socket reader wait for provider discovery or account probes.
#[derive(Default)]
struct Lane {
    turn: Mutex<u64>,
    ready: Condvar,
}

struct Turn<'a>(&'a Lane);

impl Lane {
    fn enter(&self, ticket: u64) -> Turn<'_> {
        let mut turn = self.turn.lock().unwrap_or_else(|p| p.into_inner());
        while *turn != ticket {
            turn = self.ready.wait(turn).unwrap_or_else(|p| p.into_inner());
        }
        Turn(self)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let mut turn = self.0.turn.lock().unwrap_or_else(|p| p.into_inner());
        *turn += 1;
        self.0.ready.notify_all();
    }
}

struct AdmissionState {
    active: usize,
    accepting: bool,
}

/// One daemon-wide bound and shutdown barrier for request tasks.
///
/// Socket readers wait here before spawning, which turns a burst into kernel
/// socket backpressure instead of an unbounded number of OS threads. Closing
/// the gate wakes blocked readers and prevents every connection from admitting
/// more work; the active count is the graceful-shutdown barrier.
struct Admission {
    state: Mutex<AdmissionState>,
    changed: Condvar,
    limit: usize,
}

struct Task(Arc<Admission>);

impl Admission {
    fn new(limit: usize) -> Self {
        assert!(limit > 0);
        Self {
            state: Mutex::new(AdmissionState {
                active: 0,
                accepting: true,
            }),
            changed: Condvar::new(),
            limit,
        }
    }

    fn begin(self: &Arc<Self>) -> Option<Task> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.accepting && state.active >= self.limit {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        if !state.accepting {
            return None;
        }
        state.active += 1;
        Some(Task(self.clone()))
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.accepting = false;
        self.changed.notify_all();
    }

    fn wait(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.active > 0 {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn close_and_wait(&self) {
        self.close();
        self.wait();
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).active
    }

    #[cfg(test)]
    fn accepting(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .accepting
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

/// Operations which address the same mutable thing share a lane. Read-only
/// global calls such as `ping`, provider discovery, and the intelligence dial
/// deliberately do not: replies carry ids and are allowed to arrive out of
/// order, so a slow account probe must not hold up an unrelated request.
fn lane_key(request: &Request) -> Option<String> {
    request
        .session
        .as_ref()
        .map(|session| format!("session:{session}"))
        .or_else(|| match request.op.as_str() {
            "account" => Some(format!(
                "account:{}",
                request.provider.as_deref().unwrap_or_default()
            )),
            "test" => Some(format!(
                "test:{}",
                request.provider.as_deref().unwrap_or("registry")
            )),
            _ => None,
        })
}

enum BoundedLine {
    Eof,
    Line,
    TooLong,
}

/// Read at most one request plus a sentinel byte. Capacity grows geometrically
/// up to that exact bound, avoiding both one huge allocation per idle socket
/// and `Vec`'s usual final doubling beyond the limit.
fn read_request_line(
    reader: &mut impl BufRead,
    line: &mut Vec<u8>,
) -> std::io::Result<BoundedLine> {
    line.clear();
    loop {
        let (used, complete) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if line.is_empty() {
                    BoundedLine::Eof
                } else {
                    BoundedLine::Line
                });
            }
            let room = MAX_REQUEST_BYTES + 1 - line.len();
            let offered = &available[..available.len().min(room)];
            let used = offered
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(offered.len(), |at| at + 1);
            let needed = line.len() + used;
            if needed > line.capacity() {
                let target = needed
                    .max(line.capacity().max(1).saturating_mul(2))
                    .min(MAX_REQUEST_BYTES + 1);
                line.reserve_exact(target - line.len());
            }
            line.extend_from_slice(&offered[..used]);
            (used, offered[..used].last() == Some(&b'\n'))
        };
        reader.consume(used);
        if complete {
            return Ok(BoundedLine::Line);
        }
        if line.len() == MAX_REQUEST_BYTES + 1 {
            return Ok(BoundedLine::TooLong);
        }
    }
}

/// Read one client's requests until it goes away.
fn talk(daemon: Arc<Daemon>, admission: Arc<Admission>, id: u64, stream: UnixStream) {
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let conn = Connection::new(id, stream);
    let mut lanes: BTreeMap<String, (Arc<Lane>, u64)> = BTreeMap::new();
    let mut reader = BufReader::new(reading);
    let mut line = Vec::new();
    loop {
        match read_request_line(&mut reader, &mut line) {
            Ok(BoundedLine::Line) => {}
            Ok(BoundedLine::Eof) => break,
            Ok(BoundedLine::TooLong) => {
                conn.reply(Reply::failed(
                    0,
                    format!("request exceeds the {MAX_REQUEST_BYTES}-byte limit"),
                ));
                let _ = conn.flush(Duration::from_secs(2));
                break;
            }
            Err(err) => {
                conn.reply(Reply::failed(0, format!("cannot read request: {err}")));
                let _ = conn.flush(Duration::from_secs(2));
                break;
            }
        }
        let line = match std::str::from_utf8(&line) {
            Ok(line) => line,
            Err(err) => {
                conn.reply(Reply::failed(0, format!("request is not UTF-8: {err}")));
                let _ = conn.flush(Duration::from_secs(2));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(line) {
            Ok(request) => {
                let stop = request.op == "shutdown";
                if stop {
                    admission.close_and_wait();
                    conn.reply(daemon.answer(&request, &conn));
                    let _ = conn.flush(Duration::from_secs(2));
                    // Poke the accept loop so it notices and unwinds.
                    let _ = UnixStream::connect(paths::socket());
                    break;
                }
                let lane = lane_key(&request).map(|key| {
                    let (lane, next) = lanes.entry(key).or_default();
                    let ticket = *next;
                    *next += 1;
                    (lane.clone(), ticket)
                });
                let Some(task) = admission.begin() else {
                    conn.reply(Reply::failed(request.id, SHUTTING_DOWN));
                    let _ = conn.flush(Duration::from_secs(2));
                    break;
                };
                let daemon = daemon.clone();
                let conn = conn.clone();
                std::thread::spawn(move || {
                    let _task = task;
                    let _turn = lane.as_ref().map(|(lane, ticket)| lane.enter(*ticket));
                    if conn.open() {
                        conn.reply(daemon.answer(&request, &conn));
                    }
                });
            }
            Err(err) => {
                conn.reply(Reply::failed(0, format!("that was not a request: {err}")));
            }
        }
    }
    // Mark the connection closed before forgetting it. A provider probe which
    // finishes after EOF must not attach a listener that has already missed
    // this cleanup pass.
    conn.close();
    daemon.registry.forget(id);
}

/// Find out which providers are usable before anybody asks.
///
/// Every probe runs a CLI, and the slowest of them takes the better part of a
/// minute. Paying that once, in the background, at the moment the daemon starts
/// is the difference between a first session that opens instantly and one that
/// sits there. This is what a daemon is *for*.
fn prime() {
    std::thread::spawn(|| {
        let usable = omni_core::providers::available(None);
        say(&format!(
            "ready: {}",
            if usable.is_empty() {
                "none logged in".into()
            } else {
                usable.join(", ")
            }
        ));
    });
}

/// Put down sessions nobody is listening to any more.
fn sweep(daemon: Arc<Daemon>) {
    std::thread::spawn(move || {
        let grace = registry::idle_grace();
        loop {
            std::thread::sleep(SWEEP);
            if daemon.stopping.load(Ordering::SeqCst) {
                return;
            }
            let gone = daemon.registry.reap(grace);
            if gone > 0 {
                say(&format!("let {gone} cold session(s) go"));
            }
            // Keep the answer to "which providers can I use" warm, so no client
            // ever waits on a CLI to say whether it is logged in.
            if omni_core::providers::probed_ago().is_none_or(|age| age > 300.0) {
                omni_core::providers::available(None);
            }
        }
    });
}

fn say(what: &str) {
    let line = format!(
        "{} omnid[{}] {what}\n",
        omni_core::shared::clock::now(),
        std::process::id()
    );
    print!("{line}");
    let _ = std::io::stdout().flush();
    if std::env::var_os("OMNI_STDIO_LOGGED").is_none() {
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(paths::log_file())
        {
            let _ = log.set_permissions(std::fs::Permissions::from_mode(0o600));
            let _ = log.write_all(line.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_for(want: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if want() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn the_daemon_lock_excludes_a_second_start_and_survives_as_a_file() {
        let path = std::env::temp_dir().join(format!(
            "omni-daemon-lock-{}-{}",
            std::process::id(),
            omni_core::shared::clock::now().replace([':', '.'], "-")
        ));
        let _ = std::fs::remove_file(&path);
        let first = lock_daemon(&path).unwrap();
        assert!(lock_daemon(&path).is_err());
        drop(first);
        let second = lock_daemon(&path).unwrap();
        assert!(path.exists(), "the reusable inode stays in place");
        drop(second);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn one_lane_is_fifo_without_blocking_an_unrelated_lane() {
        let first = Arc::new(Lane::default());
        let second = Arc::new(Lane::default());
        let (done, seen) = mpsc::channel();

        let held = first.enter(0);
        let waiting = first.clone();
        let later = done.clone();
        std::thread::spawn(move || {
            let _turn = waiting.enter(1);
            later.send("same").unwrap();
        });
        let independent = second.clone();
        let earlier = done.clone();
        std::thread::spawn(move || {
            let _turn = independent.enter(0);
            earlier.send("other").unwrap();
        });

        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), "other");
        drop(held);
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), "same");
    }

    #[test]
    fn admission_caps_tasks_and_wakes_waiters_on_drop_or_close() {
        let admission = Arc::new(Admission::new(1));
        let first = admission.begin().unwrap();
        let waiting = admission.clone();
        let (acquired, seen) = mpsc::channel();
        std::thread::spawn(move || {
            acquired.send(waiting.begin()).unwrap();
        });

        assert!(seen.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        let second = seen.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();

        let waiting = admission.clone();
        let (refused, seen) = mpsc::channel();
        std::thread::spawn(move || {
            refused.send(waiting.begin()).unwrap();
        });
        assert!(seen.recv_timeout(Duration::from_millis(50)).is_err());
        admission.close();
        assert!(seen.recv_timeout(Duration::from_secs(1)).unwrap().is_none());
        drop(second);
        admission.wait();
        assert_eq!(admission.active(), 0);
        assert!(admission.begin().is_none());
    }

    #[test]
    fn a_slow_request_does_not_hold_up_an_unrelated_reply_on_one_socket() {
        let (server, mut client) = UnixStream::pair().unwrap();
        let daemon = Arc::new(Daemon::default());
        let admission = Arc::new(Admission::new(MAX_IN_FLIGHT));
        let serving = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 1, server))
        };

        let slow = serde_json::to_string(
            &Request::new(1, "test").about("sleep", serde_json::json!({"milliseconds": 200})),
        )
        .unwrap();
        let ping = serde_json::to_string(&Request::new(2, "ping")).unwrap();
        writeln!(client, "{slow}").unwrap();
        writeln!(client, "{ping}").unwrap();
        client.flush().unwrap();

        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let first: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(first.id, 2, "ping waited behind an unrelated slow call");
        line.clear();
        reader.read_line(&mut line).unwrap();
        let second: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(second.id, 1);

        client.shutdown(std::net::Shutdown::Both).unwrap();
        serving.join().unwrap();
        daemon.registry.shutdown();
    }

    #[test]
    fn an_oversized_line_closes_only_its_connection() {
        let daemon = Arc::new(Daemon::default());
        let admission = Arc::new(Admission::new(MAX_IN_FLIGHT));
        let (server, mut client) = UnixStream::pair().unwrap();
        let serving = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 1, server))
        };

        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let oversized = vec![b' '; MAX_REQUEST_BYTES + 1];
        let _ = client.write_all(&oversized);
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let rejected: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(rejected.id, 0);
        assert!(!rejected.ok);
        assert!(rejected.error.unwrap().contains("byte limit"));
        serving.join().unwrap();

        let (server, mut client) = UnixStream::pair().unwrap();
        let serving = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 2, server))
        };
        writeln!(
            client,
            "{}",
            serde_json::to_string(&Request::new(7, "ping")).unwrap()
        )
        .unwrap();
        client.flush().unwrap();
        let mut line = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let reply: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(reply.id, 7);
        assert!(reply.ok, "another connection remained usable");
        client.shutdown(std::net::Shutdown::Both).unwrap();
        serving.join().unwrap();
        daemon.registry.shutdown();
    }

    #[test]
    fn shutdown_is_a_barrier_across_connections_and_closes_admission() {
        let _home = omni_core::testing::scratch_home("daemon-global-shutdown");
        let daemon = Arc::new(Daemon::default());
        let admission = Arc::new(Admission::new(MAX_IN_FLIGHT));
        let (slow_server, mut slow_client) = UnixStream::pair().unwrap();
        let (stop_server, mut stop_client) = UnixStream::pair().unwrap();
        slow_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stop_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let slow_talk = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 1, slow_server))
        };
        let stop_talk = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 2, stop_server))
        };

        let slow = serde_json::to_string(
            &Request::new(1, "test").about("sleep", serde_json::json!({"milliseconds": 500})),
        )
        .unwrap();
        writeln!(slow_client, "{slow}").unwrap();
        slow_client.flush().unwrap();
        assert!(wait_for(|| admission.active() == 1));

        let shutdown = serde_json::to_string(&Request::new(2, "shutdown")).unwrap();
        writeln!(stop_client, "{shutdown}").unwrap();
        stop_client.flush().unwrap();
        assert!(wait_for(|| !admission.accepting()));

        let (late_server, mut late_client) = UnixStream::pair().unwrap();
        late_client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let late_talk = {
            let daemon = daemon.clone();
            let admission = admission.clone();
            std::thread::spawn(move || talk(daemon, admission, 3, late_server))
        };
        writeln!(
            late_client,
            "{}",
            serde_json::to_string(&Request::new(3, "ping")).unwrap()
        )
        .unwrap();
        late_client.flush().unwrap();
        let mut late_line = String::new();
        BufReader::new(late_client.try_clone().unwrap())
            .read_line(&mut late_line)
            .unwrap();
        let late: Reply = serde_json::from_str(&late_line).unwrap();
        assert_eq!(late.id, 3);
        assert_eq!(late.error.as_deref(), Some(SHUTTING_DOWN));
        late_talk.join().unwrap();

        let (stopped, seen) = mpsc::channel();
        let stop_reader = stop_client.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stop_reader).read_line(&mut line).unwrap();
            stopped
                .send(serde_json::from_str::<Reply>(&line).unwrap())
                .unwrap();
        });
        assert!(
            seen.recv_timeout(Duration::from_millis(100)).is_err(),
            "shutdown overtook work accepted on another connection"
        );

        let mut slow_line = String::new();
        BufReader::new(slow_client.try_clone().unwrap())
            .read_line(&mut slow_line)
            .unwrap();
        assert_eq!(serde_json::from_str::<Reply>(&slow_line).unwrap().id, 1);
        let stopped = seen.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(stopped.id, 2);
        assert!(stopped.ok);

        slow_client.shutdown(std::net::Shutdown::Both).unwrap();
        let _ = stop_client.shutdown(std::net::Shutdown::Both);
        slow_talk.join().unwrap();
        stop_talk.join().unwrap();
        daemon.registry.shutdown();
    }
}
