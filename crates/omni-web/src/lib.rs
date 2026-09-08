//! `omni web` — a loopback door onto the omni daemon, for browsers.
//!
//! The daemon speaks over a Unix socket, which a webpage cannot open. This
//! crate puts an HTTP/1.1 server in front of it on `127.0.0.1`, with an event
//! stream for watching a turn happen and a pairing step so that being able to
//! reach the port is not the same as being allowed through it.
//!
//! Three commands, and they are the whole story:
//!
//! ```text
//! omni web            # the bridge, in the foreground, on 1998 if it can
//! omni web connect    # a code to carry to a website: XULA-1998
//! omni web status     # who is paired, and what they are watching
//! ```
//!
//! The port is part of the identity. A browser paired with `localhost:1998`,
//! and if the bridge comes back somewhere else that is a different address
//! with, as far as the browser can tell, a different thing behind it. So a
//! move clears every grant and everyone pairs again. Coming back up on the
//! same port keeps them, which is why the last one is written down.

pub mod api;
pub mod bridge;
pub mod http;
pub mod keys;
pub mod sha256;
pub mod state;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use api::Web;
use bridge::Bridge;
use http::Server;
use state::Doorman;

pub const HELP: &str = r#"omni web — one localhost door onto omni, for websites

Usage:
  omni web [--port N]            run the bridge in the foreground
  omni web connect [--name WHO]  show a pairing code for a website
  omni web status [--json]       the port, who is paired, what is streaming
  omni web revoke WHO | --all    take a website's access back
  omni web stop                  stop the running bridge

The bridge listens on 127.0.0.1:1998, counting down to 1989 if that is taken.
A website finds it by asking each of those ports for GET /omni.

Pairing:
  `omni web connect` prints a code like XULA-1998 — four letters and the port.
  A website exchanges it once, at POST /connect, for an omniauth. The code is
  spent on first use and expires in five minutes; the omniauth lasts an hour
  from its last request and a day from when it was issued.

  Grants belong to a port. Restart the bridge on the same one and everybody
  stays paired; land anywhere else and everybody pairs again.

Environment:
  OMNI_HOME    state directory (default: ~/.omni), which holds web.json
"#;

/// One line of bridge chatter, on stderr so `--json` output stays clean.
pub fn say(message: &str) {
    eprintln!("omni web · {message}");
}

/// Run an `omni web ...` command. `args` is everything after `web`.
pub fn main(mut args: Vec<String>) -> Result<(), String> {
    let action = args.first().cloned().unwrap_or_default();
    if action.starts_with('-') && !matches!(action.as_str(), "-h" | "--help") {
        // `omni web --port 1990` is the server with an option, not an action.
        return run(take_port(&mut args)?);
    }
    if !args.is_empty() {
        args.remove(0);
    }
    match action.as_str() {
        "" => run(take_port(&mut args)?),
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        "connect" | "pair" => connect(take_value(&mut args, "--name")?, take_flag(&mut args, "--json")),
        "status" => status(take_flag(&mut args, "--json")),
        "revoke" => revoke(args),
        "stop" => stop(),
        other => Err(format!(
            "unknown web command {other:?}; try `omni web help`"
        )),
    }
}

// ------------------------------------------------------------------ the server

/// Bind a port, remember it, and answer until told to stop.
pub fn run(pin: Option<u16>) -> Result<(), String> {
    let remembered = state::load().port;
    let server = match pin {
        // An explicit port is an instruction, not a preference. Falling back
        // from one would put the bridge somewhere the caller did not ask for
        // and quietly invalidate every grant while doing it.
        Some(port) => Server::bind(port)
            .map_err(|error| format!("cannot listen on 127.0.0.1:{port}: {error}"))?,
        None => Server::find(remembered)?,
    };
    let port = server.port;

    // Start the daemon now rather than on the first request, so a website's
    // first call is not the one that waits for three CLIs to come up.
    match silicon_omni::ensure_daemon(silicon_omni::STARTUP_TIMEOUT) {
        Ok(_) => {}
        Err(error) => say(&format!("the daemon is not up yet ({error}); trying again on demand")),
    }

    let doorman = Arc::new(Doorman::open(state::path(), port));
    let bridge = Arc::new(Bridge::default());
    let web = Arc::new(Web::new(doorman.clone(), bridge.clone()));
    let stopper = server.stopper();
    *web.stopper.lock().unwrap_or_else(|p| p.into_inner()) = Some(stopper.clone());

    say(&format!("listening on http://127.0.0.1:{port}"));
    match (remembered, doorman.carried_over()) {
        (Some(was), _) if was != port => say(&format!(
            "this is not {was}, where it was last time, so every previous pairing is cleared"
        )),
        (_, 0) => say("nothing is paired yet — run `omni web connect` for a code"),
        (_, 1) => say("1 site is still paired from last time"),
        (_, held) => say(&format!("{held} sites are still paired from last time")),
    }

    let watching = watch_signals(&stopper);
    let sweeping = sweep(doorman.clone(), bridge.clone(), stopper.clone());

    let handler = {
        let web = web.clone();
        Arc::new(move |request| Web::answer(&web, request))
    };
    server.run(handler);

    stopper.stop();
    let _ = sweeping.join();
    drop(watching);
    bridge.forget_all();
    doorman.release();
    say("stopped");
    Ok(())
}

/// Ctrl-C and `kill` come out where a `stop` request does, so the port and
/// the grants are written down either way.
fn watch_signals(stopper: &http::Stopper) -> Option<std::thread::JoinHandle<()>> {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::signal::SIGINT,
        signal_hook::consts::signal::SIGTERM,
    ])
    .ok()?;
    let handle = signals.handle();
    let stopper = stopper.clone();
    let thread = std::thread::spawn(move || {
        if signals.forever().next().is_some() {
            say("stopping");
            stopper.stop();
        }
        handle.close();
    });
    Some(thread)
}

/// Let go of what has run out: expired grants, and the daemon attachments
/// that were only being held for a reader who left.
fn sweep(
    doorman: Arc<Doorman>,
    bridge: Arc<Bridge>,
    stopper: http::Stopper,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stopper.stopping() {
            for grant in doorman.sweep() {
                bridge.forget(&grant);
            }
            bridge.sweep();
            for _ in 0..30 {
                if stopper.stopping() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    })
}

// ----------------------------------------------------------------- the client

fn connect(name: Option<String>, as_json: bool) -> Result<(), String> {
    let name = name.unwrap_or_default();
    let answer = control("POST", "/control/connect", &json!({"name": name}))?;
    let code = answer["code"].as_str().unwrap_or_default();
    let port = answer["bridge"]["port"].as_i64().unwrap_or_default();
    if as_json {
        println!("{}", serde_json::to_string_pretty(&answer).unwrap_or_default());
        return Ok(());
    }
    println!("{code}");
    println!();
    println!("  Paste it into the site you are connecting. It works once, and");
    println!("  only for the next {:.0} minutes.", state::CODE_TTL / 60.0);
    println!("  The site will find the bridge at http://localhost:{port}.");
    Ok(())
}

fn status(as_json: bool) -> Result<(), String> {
    let answer = match control("GET", "/control/status", &Value::Null) {
        Ok(answer) => answer,
        Err(why) if !as_json => return Err(why),
        Err(why) => {
            println!("{}", json!({"running": false, "why": why}));
            return Ok(());
        }
    };
    if as_json {
        println!("{}", serde_json::to_string_pretty(&answer).unwrap_or_default());
        return Ok(());
    }
    let bridge = &answer["bridge"];
    println!(
        "running on {} (pid {}, v{}, since {})",
        bridge["port"],
        answer["pid"],
        bridge["version"].as_str().unwrap_or_default(),
        bridge["started"].as_str().unwrap_or_default()
    );
    println!(
        "{} streaming, {} served",
        answer["streaming"], answer["served"]
    );
    let grants = answer["grants"].as_array().cloned().unwrap_or_default();
    if grants.is_empty() {
        println!("nothing is paired — run `omni web connect` for a code");
        return Ok(());
    }
    println!("id\torigin\tname\texpires in");
    for grant in grants {
        println!(
            "{}\t{}\t{}\t{:.0}m",
            grant["id"].as_str().unwrap_or_default(),
            blank(grant["origin"].as_str().unwrap_or_default(), "(no browser)"),
            blank(grant["name"].as_str().unwrap_or_default(), "-"),
            grant["expires_in"].as_f64().unwrap_or_default() / 60.0
        );
    }
    Ok(())
}

fn revoke(args: Vec<String>) -> Result<(), String> {
    let all = args.iter().any(|arg| arg == "--all");
    let which = args.iter().find(|arg| !arg.starts_with('-')).cloned();
    if !all && which.is_none() {
        return Err("revoke needs a grant id, an origin, or --all".into());
    }
    let answer = control(
        "POST",
        "/control/revoke",
        &json!({"all": all, "which": which.unwrap_or_default()}),
    )?;
    let taken = answer["revoked"].as_array().cloned().unwrap_or_default();
    if taken.is_empty() {
        println!("nothing matched");
        return Ok(());
    }
    for grant in taken {
        println!(
            "revoked {} {}",
            grant["id"].as_str().unwrap_or_default(),
            blank(grant["origin"].as_str().unwrap_or_default(), "(no browser)")
        );
    }
    Ok(())
}

fn stop() -> Result<(), String> {
    match control("POST", "/control/stop", &Value::Null) {
        Ok(_) => {
            println!("stopping");
            Ok(())
        }
        Err(why) if why.contains("no bridge") => {
            println!("already stopped");
            Ok(())
        }
        Err(why) => Err(why),
    }
}

fn blank<'a>(text: &'a str, instead: &'a str) -> &'a str {
    if text.is_empty() { instead } else { text }
}

/// Ask the running bridge something, as whoever can read `web.json`.
fn control(method: &str, path: &str, body: &Value) -> Result<Value, String> {
    let memory = state::load();
    let (Some(port), false) = (memory.port, memory.control.is_empty()) else {
        return Err(no_bridge());
    };
    if memory.pid.is_none() {
        // The file says the last bridge stood down cleanly. Knock anyway —
        // one that was killed leaves the same trace — but say the right thing
        // if nothing answers.
        return request(port, method, path, &memory.control, body)
            .map_err(|why| if why.contains("connect") { no_bridge() } else { why });
    }
    request(port, method, path, &memory.control, body)
        .map_err(|why| if why.contains("connect") { no_bridge() } else { why })
}

fn no_bridge() -> String {
    "no bridge is running; start one with `omni web`".into()
}

/// A very small HTTP client. It only ever talks to the bridge it just found
/// on loopback, so there is no redirect, no TLS and no chunked reply to
/// handle — and pulling in something that does all three to make one POST to
/// 127.0.0.1 would be the wrong shape of dependency.
fn request(
    port: u16,
    method: &str,
    path: &str,
    control: &str,
    body: &Value,
) -> Result<Value, String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(5))
        .map_err(|error| format!("cannot connect to 127.0.0.1:{port}: {error}"))?;
    socket
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;

    let payload = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(body).unwrap_or_default()
    };
    let head = format!(
        "{method} {path} HTTP/1.1\r\n\
         host: 127.0.0.1:{port}\r\n\
         authorization: Bearer {control}\r\n\
         content-type: application/json\r\n\
         content-length: {}\r\n\
         connection: close\r\n\r\n",
        payload.len()
    );
    socket
        .write_all(head.as_bytes())
        .and_then(|_| socket.write_all(&payload))
        .and_then(|_| socket.flush())
        .map_err(|error| format!("cannot ask the bridge: {error}"))?;

    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("the bridge said nothing: {error}"))?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("the bridge answered {line:?}, which is not HTTP"))?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
    }
    let mut rest = String::new();
    reader
        .read_to_string(&mut rest)
        .map_err(|error| format!("the bridge stopped half way: {error}"))?;
    let answer: Value = serde_json::from_str(rest.trim()).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(answer);
    }
    Err(answer["error"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("the bridge answered {status}")))
}

// ------------------------------------------------------------------ arguments

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|arg| arg == name) {
        Some(at) => {
            args.remove(at);
            true
        }
        None => false,
    }
}

fn take_value(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(at) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    args.remove(at);
    if at >= args.len() {
        return Err(format!("{name} needs a value"));
    }
    Ok(Some(args.remove(at)))
}

fn take_port(args: &mut Vec<String>) -> Result<Option<u16>, String> {
    let Some(raw) = take_value(args, "--port")? else {
        return Ok(None);
    };
    let port: u16 = raw
        .parse()
        .map_err(|_| format!("--port takes a port number, not {raw:?}"))?;
    if port == 0 {
        return Err("--port needs a real port".into());
    }
    Ok(Some(port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_shows_all_three_commands_and_the_port_it_looks_for() {
        assert!(HELP.contains("omni web connect"));
        assert!(HELP.contains("omni web status"));
        assert!(HELP.contains("127.0.0.1:1998"));
        assert!(HELP.contains("GET /omni"), "how a website finds it");
    }

    #[test]
    fn a_bare_option_still_means_run_the_server() {
        let mut args = vec!["--port".to_string(), "1990".to_string()];
        assert_eq!(take_port(&mut args).unwrap(), Some(1990));
        assert!(args.is_empty());
    }

    #[test]
    fn a_port_that_is_not_one_is_refused_rather_than_rounded() {
        let mut args = vec!["--port".to_string(), "nope".to_string()];
        assert!(take_port(&mut args).unwrap_err().contains("port number"));
        let mut args = vec!["--port".to_string()];
        assert!(take_port(&mut args).unwrap_err().contains("needs a value"));
    }

    #[test]
    fn an_unknown_command_names_the_help_rather_than_guessing() {
        let error = main(vec!["frobnicate".into()]).unwrap_err();
        assert!(error.contains("frobnicate"));
        assert!(error.contains("omni web help"));
    }
}
