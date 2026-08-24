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

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use omni_core::shared::paths;
use omni_core::wire::{Reply, Request};

use serve::Daemon;
use wiring::Connection;

/// How often to look for sessions nobody wants any more.
const SWEEP: Duration = Duration::from_secs(30);

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
    claim(&socket)?;
    let listener =
        UnixListener::bind(&socket).map_err(|err| format!("cannot listen on {socket:?}: {err}"))?;
    let _ = std::fs::write(paths::daemon_lock(), std::process::id().to_string());
    say(&format!("listening on {}", socket.display()));

    let daemon = Arc::new(Daemon::default());
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
                std::thread::spawn(move || talk(daemon, id, stream));
            }
            Err(err) => say(&format!("a connection went wrong: {err}")),
        }
    }
    say("shutting down");
    daemon.registry.shutdown();
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(paths::daemon_lock());
    Ok(())
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

/// Read one client's requests until it goes away.
fn talk(daemon: Arc<Daemon>, id: u64, stream: UnixStream) {
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let conn = Connection::new(id, stream);
    for line in BufReader::new(reading).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                let stop = request.op == "shutdown";
                conn.reply(daemon.answer(&request, &conn));
                if stop {
                    // Poke the accept loop so it notices and unwinds.
                    let _ = UnixStream::connect(paths::socket());
                    break;
                }
            }
            Err(err) => conn.reply(Reply::failed(0, format!("that was not a request: {err}"))),
        }
    }
    daemon.registry.forget(id);
    conn.close();
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
    use std::io::Write;
    let _ = std::io::stdout().flush();
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_file())
    {
        let _ = log.write_all(line.as_bytes());
    }
}
