//! `omni` — a thin terminal client for `omnid`.

use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::time::Duration;

use omni_client::{Client, Event, Frame, OpenOptions, Request, Session, Snapshot, kind};
use serde_json::{Value, json};

const HELP: &str = r#"omni — one conversation across Claude, Codex, and Antigravity

Usage:
  omni chat [OPTIONS] SESSION [MESSAGE...]    interactive or one-turn chat
  omni send [OPTIONS] SESSION MESSAGE...      send and stream until settled
  omni attach [OPTIONS] SESSION               replay/follow a session
  omni events [--from SEQ] [--json] SESSION   read persisted events once
  omni status SESSION                         show a live session snapshot
  omni sessions                               list live sessions
  omni stop SESSION                           stop a live session
  omni set SESSION SETTING VALUE              queue a setting change
  omni providers [PROVIDER...]                list available providers
  omni dial [PROVIDER...]                     show the 0–10 intelligence dial
  omni account PROVIDER ACTION [CODE]          account status/login/limits
  omni daemon start|status|stop                manage the persistent daemon
  omni ping                                    show daemon information
  omni request OP [JSON]                       make a low-level protocol call

Stream options:
  --from SEQ          first event sequence to replay (send/chat default: -1)
  --providers A,B     limit providers when opening the session
  --level 0..10       set intelligence as part of opening
  --json              emit machine-readable JSON lines

Account actions:
  status (default), installed, limits, login, finish CODE, forget

Environment:
  OMNI_HOME           state directory (default: ~/.omni)
  OMNI_DAEMON         exact omnid binary to start
  OMNI_REGISTRY       intelligence registry endpoint
"#;

#[derive(Debug)]
struct CliError(String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CliError {}

impl From<omni_client::Error> for CliError {
    fn from(error: omni_client::Error) -> Self {
        CliError(error.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(error: serde_json::Error) -> Self {
        CliError(error.to_string())
    }
}

impl From<io::Error> for CliError {
    fn from(error: io::Error) -> Self {
        CliError(error.to_string())
    }
}

type Result<T> = std::result::Result<T, CliError>;

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("omni: {error}");
        std::process::exit(1);
    }
}

fn run(mut args: Vec<String>) -> Result<()> {
    let Some(command) = args.first().cloned() else {
        print!("{HELP}");
        return Ok(());
    };
    args.remove(0);
    match command.as_str() {
        "help" | "-h" | "--help" => print!("{HELP}"),
        "version" | "-V" | "--version" => println!("omni {}", env!("CARGO_PKG_VERSION")),
        "ping" => print_value(&serde_json::to_value(Client::connect()?.ping()?)?),
        "daemon" => daemon(args)?,
        "providers" => providers(args)?,
        "dial" => dial(args)?,
        "sessions" => sessions(no_args(command, args)?)?,
        "status" => status(one_arg(command, args)?)?,
        "events" => events(args)?,
        "attach" => attach(args)?,
        "send" => stream(args, false)?,
        "chat" => stream(args, true)?,
        "stop" => stop(one_arg(command, args)?)?,
        "set" => set(args)?,
        "account" => account(args)?,
        "request" => request(args)?,
        other => {
            return Err(CliError(format!(
                "unknown command {other:?}; try `omni help`"
            )));
        }
    }
    Ok(())
}

fn daemon(mut args: Vec<String>) -> Result<()> {
    let action = if args.is_empty() {
        "status".to_string()
    } else {
        args.remove(0)
    };
    no_args("daemon", args)?;
    match action.as_str() {
        "status" => match Client::connect_existing() {
            Ok(client) => {
                let info = client.ping()?;
                println!(
                    "running (pid {}, v{}, protocol {}, since {})",
                    info.pid, info.version, info.protocol, info.started
                );
            }
            Err(_) => println!("stopped"),
        },
        "start" => {
            let info = Client::connect()?.ping()?;
            println!("running (pid {}, v{})", info.pid, info.version);
        }
        "stop" => match Client::connect_existing() {
            Ok(client) => {
                client.shutdown()?;
                if omni_client::wait_until_stopped(Duration::from_secs(90)) {
                    println!("stopped");
                } else {
                    return Err(CliError("daemon did not stop within 90s".into()));
                }
            }
            Err(_) => println!("already stopped"),
        },
        other => {
            return Err(CliError(format!(
                "unknown daemon action {other:?}; use start, status, or stop"
            )));
        }
    }
    Ok(())
}

fn providers(args: Vec<String>) -> Result<()> {
    reject_options(&args)?;
    let only = (!args.is_empty()).then_some(args);
    for provider in Client::connect()?.providers(only)? {
        println!("{provider}");
    }
    Ok(())
}

fn dial(args: Vec<String>) -> Result<()> {
    reject_options(&args)?;
    let only = (!args.is_empty()).then_some(args);
    let table = Client::connect()?.dial(only)?;
    println!("level  provider  model  effort");
    for (level, rung) in table.iter().rev() {
        println!(
            "{level:>5}  {:<8}  {}  {}",
            rung.provider, rung.model, rung.effort
        );
    }
    Ok(())
}

fn sessions(_: ()) -> Result<()> {
    let sessions = Client::connect()?.sessions()?;
    if sessions.is_empty() {
        return Ok(());
    }
    println!("session\tstatus\tprovider\tmodel\tlevel\tlisteners");
    for live in sessions {
        let state = live.snapshot;
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            state.session, state.status, state.provider, state.model, state.level, live.listeners
        );
    }
    Ok(())
}

fn status(session: String) -> Result<()> {
    let (snapshot, listeners) = Client::connect()?.status(&session)?;
    print_value(&json!({"snapshot": snapshot, "listeners": listeners}));
    Ok(())
}

fn events(mut args: Vec<String>) -> Result<()> {
    let json = take_flag(&mut args, "--json");
    let from = take_i64(&mut args, "--from")?.unwrap_or(0);
    let session = one_arg("events", args)?;
    for event in Client::connect()?.events(&session, from)? {
        print_event(&event, json)?;
    }
    Ok(())
}

fn attach(mut args: Vec<String>) -> Result<()> {
    let json = take_flag(&mut args, "--json");
    let from = take_i64(&mut args, "--from")?.unwrap_or(0);
    let providers = take_providers(&mut args)?;
    let session_id = one_arg("attach", args)?;
    let client = Client::connect()?;
    let mut options = OpenOptions::new(&session_id).from_seq(from);
    if let Some(providers) = providers {
        options = options.providers(providers);
    }
    let mut session = client.open(options)?;
    loop {
        let frame = session.recv()?;
        let gone = frame.stream == "gone";
        print_frame(&frame, json)?;
        if gone {
            break;
        }
    }
    let _ = session.detach();
    Ok(())
}

fn stream(mut args: Vec<String>, interactive: bool) -> Result<()> {
    let json = take_flag(&mut args, "--json");
    let from = take_i64(&mut args, "--from")?.unwrap_or(-1);
    let level = take_i64(&mut args, "--level")?;
    if level.is_some_and(|level| !(0..=10).contains(&level)) {
        return Err(CliError("--level must be between 0 and 10".into()));
    }
    let providers = take_providers(&mut args)?;
    reject_options(&args)?;
    if args.is_empty() {
        return Err(CliError(format!(
            "{} needs a session{}",
            if interactive { "chat" } else { "send" },
            if interactive { "" } else { " and a message" }
        )));
    }
    let session_id = args.remove(0);
    let initial = args.join(" ");
    if !interactive && initial.is_empty() {
        return Err(CliError("send needs a message".into()));
    }

    let client = Client::connect()?;
    let mut options = OpenOptions::new(session_id).from_seq(from);
    if let Some(providers) = providers {
        options = options.providers(providers);
    }
    if let Some(level) = level {
        options = options.setting("level", level);
    }
    let mut session = client.open(options)?;

    // `open` may replay before its reply. Show those frames now so an old END
    // cannot be mistaken for the boundary of the message about to be sent.
    for _ in 0..session.opened().replayed {
        print_frame(&session.recv()?, json)?;
    }

    if !initial.is_empty() {
        send_and_wait(&session, &initial, json)?;
    }
    if interactive {
        repl(&session, json)?;
    }
    session.detach()?;
    Ok(())
}

fn repl(session: &Session, json: bool) -> Result<()> {
    let terminal = io::stdin().is_terminal();
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        if terminal {
            print!("> ");
            io::stdout().flush()?;
        }
        let Some(line) = lines.next() else {
            break;
        };
        let line = line?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if matches!(text, "/quit" | "/exit") {
            break;
        }
        send_and_wait(session, text, json)?;
    }
    Ok(())
}

fn send_and_wait(session: &Session, text: &str, json: bool) -> Result<()> {
    if !session.send(text)? {
        return Err(CliError("daemon did not accept the message".into()));
    }
    let mut acknowledged = false;
    let mut failure = None;
    loop {
        let frame = session.recv()?;
        let event = frame.event.as_ref();
        if event.is_some_and(|event| {
            matches!(event.kind.as_str(), kind::START | kind::INJECTED) && event.text == text
        }) {
            acknowledged = true;
        }
        if let Some(error) = event.filter(|event| {
            event.kind == kind::ERROR
                && event.fault != "stderr"
                && event.extra.get("willRetry").and_then(Value::as_bool) != Some(true)
        }) {
            failure = Some(if error.error.is_empty() {
                "inference failed".to_string()
            } else {
                error.error.clone()
            });
        }
        // A rejected send deliberately remains queued for a later retry, so
        // `queued == 0` cannot be required here. Once the daemon says it is no
        // longer in a turn, an error is the terminal answer to this attempt.
        let terminal_error = failed_attempt(event, frame.snapshot.as_ref());
        let settled_after_message = acknowledged && settled(frame.snapshot.as_ref());
        let gone = frame.stream == "gone";
        print_frame(&frame, json)?;
        if terminal_error {
            return Err(CliError(
                failure.unwrap_or_else(|| "the message could not be delivered".into()),
            ));
        }
        if gone {
            return match failure {
                Some(error) => Err(CliError(error)),
                None => Ok(()),
            };
        }
        if settled_after_message {
            if let Some(error) = failure {
                return Err(CliError(error));
            }
            return Ok(());
        }
    }
}

fn settled(snapshot: Option<&Snapshot>) -> bool {
    snapshot.is_some_and(|state| state.status == "waiting" && !state.in_turn && state.queued == 0)
}

fn failed_attempt(event: Option<&Event>, snapshot: Option<&Snapshot>) -> bool {
    event.is_some_and(|event| event.kind == kind::ERROR)
        && snapshot.is_some_and(|state| {
            !state.in_turn && matches!(state.status.as_str(), "waiting" | "stopped")
        })
}

fn stop(session: String) -> Result<()> {
    println!(
        "{}",
        if Client::connect()?.stop(&session)? {
            "stopped"
        } else {
            "not live"
        }
    );
    Ok(())
}

fn set(mut args: Vec<String>) -> Result<()> {
    reject_options(&args)?;
    if args.len() < 3 {
        return Err(CliError("set needs SESSION SETTING VALUE".into()));
    }
    let session = args.remove(0);
    let what = args.remove(0);
    let value = parse_setting(&what, &args.join(" "))?;
    if Client::connect()?.set(&session, &what, value)? {
        println!("accepted");
    } else {
        return Err(CliError("session is no longer accepting changes".into()));
    }
    Ok(())
}

fn account(mut args: Vec<String>) -> Result<()> {
    reject_options(&args)?;
    if args.is_empty() {
        return Err(CliError("account needs a provider".into()));
    }
    let provider = args.remove(0);
    let action = if args.is_empty() {
        "status".to_string()
    } else {
        args.remove(0)
    };
    let (what, text) = match action.as_str() {
        "status" => ("auth_status", None),
        "installed" => ("installed", None),
        "limits" => ("limits", None),
        "login" | "start" => ("start_auth", None),
        "finish" => {
            if args.is_empty() {
                return Err(CliError(
                    "account finish needs a code or redirect URL".into(),
                ));
            }
            ("finish_auth", Some(args.join(" ")))
        }
        "forget" => ("forget", None),
        other => {
            return Err(CliError(format!(
                "unknown account action {other:?}; use status, installed, limits, login, finish, or forget"
            )));
        }
    };
    if text.is_none() && !args.is_empty() {
        return Err(CliError(format!(
            "account {action} takes no extra arguments"
        )));
    }
    let result = Client::connect()?.account(&provider, what, text.as_deref())?;
    print_value(&result);
    Ok(())
}

fn request(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        return Err(CliError("request needs an operation".into()));
    }
    let op = args.remove(0);
    if args.len() > 1 {
        return Err(CliError("request takes at most one JSON object".into()));
    }
    let fields = match args.pop() {
        Some(raw) => serde_json::from_str::<Value>(&raw)?,
        None => json!({}),
    };
    let Some(mut body) = fields.as_object().cloned() else {
        return Err(CliError("request fields must be a JSON object".into()));
    };
    body.insert("id".into(), json!(0));
    body.insert("op".into(), json!(op));
    let request: Request = serde_json::from_value(Value::Object(body))?;
    print_value(&Client::connect()?.call(request)?);
    Ok(())
}

fn print_frame(frame: &Frame, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(frame)?);
    } else if let Some(event) = frame.event.as_ref() {
        print_event(event, false)?;
    } else if frame.stream == "gone" {
        eprintln!("[session stopped]");
    }
    Ok(())
}

fn print_event(event: &Event, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(event)?);
        return Ok(());
    }
    match event.kind.as_str() {
        kind::TEXT => {
            println!("{}", event.text);
            io::stdout().flush()?;
        }
        kind::START => eprintln!("> {}", event.text),
        kind::INJECTED => eprintln!(">> {}", event.text),
        kind::THINKING => eprintln!("[thinking]"),
        kind::TOOL_CALL => eprintln!(
            "[tool {} {}]",
            event.tool,
            serde_json::to_string(&event.args)?
        ),
        kind::TOOL_RESULT => eprintln!(
            "[tool {} {}]",
            event.tool,
            if event.ok { "ok" } else { "failed" }
        ),
        kind::ERROR => eprintln!(
            "[error{}] {}",
            if event.fault.is_empty() {
                String::new()
            } else {
                format!("/{}", event.fault)
            },
            event.error
        ),
        kind::SWITCH_PROVIDER => eprintln!("[provider → {}]", event.provider),
        kind::NEW_SESSION => eprintln!("[new native session: {}]", event.provider),
        kind::CONFIG => eprintln!("[config: {}]", event.text),
        kind::END => {}
        other => eprintln!("[{other}]"),
    }
    Ok(())
}

fn print_value(value: &Value) {
    match value {
        Value::String(text) => println!("{text}"),
        _ => println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_default()
        ),
    }
}

fn parse_setting(what: &str, raw: &str) -> Result<Value> {
    if what == "providers" && !raw.trim_start().starts_with('[') {
        return Ok(json!(split_providers(raw)));
    }
    if matches!(what, "level" | "intelligence") {
        return raw
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| CliError(format!("{what} takes a number")));
    }
    if matches!(what, "subagents" | "mcp" | "autoremove") {
        return raw
            .parse::<bool>()
            .map(Value::from)
            .map_err(|_| CliError(format!("{what} takes true or false")));
    }
    Ok(serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.into())))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == name) {
        args.remove(index);
        true
    } else {
        false
    }
}

fn take_value(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    args.remove(index);
    if index >= args.len() {
        return Err(CliError(format!("{name} needs a value")));
    }
    Ok(Some(args.remove(index)))
}

fn take_i64(args: &mut Vec<String>, name: &str) -> Result<Option<i64>> {
    take_value(args, name)?
        .map(|raw| {
            raw.parse()
                .map_err(|_| CliError(format!("{name} takes a whole number, not {raw:?}")))
        })
        .transpose()
}

fn take_providers(args: &mut Vec<String>) -> Result<Option<Vec<String>>> {
    Ok(take_value(args, "--providers")?.map(|raw| split_providers(&raw)))
}

fn split_providers(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn reject_options(args: &[String]) -> Result<()> {
    if let Some(option) = args.iter().find(|arg| arg.starts_with('-')) {
        return Err(CliError(format!("unknown option {option:?}")));
    }
    Ok(())
}

fn one_arg(command: impl AsRef<str>, args: Vec<String>) -> Result<String> {
    reject_options(&args)?;
    if args.len() != 1 {
        return Err(CliError(format!(
            "{} needs exactly one session",
            command.as_ref()
        )));
    }
    Ok(args.into_iter().next().unwrap_or_default())
}

fn no_args(command: impl AsRef<str>, args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(CliError(format!("{} takes no arguments", command.as_ref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_can_appear_before_or_after_positionals() {
        let mut args = vec!["demo".into(), "--from".into(), "12".into(), "--json".into()];
        assert!(take_flag(&mut args, "--json"));
        assert_eq!(take_i64(&mut args, "--from").unwrap(), Some(12));
        assert_eq!(args, vec!["demo"]);
    }

    #[test]
    fn setting_values_follow_the_wire_types() {
        assert_eq!(parse_setting("level", "7").unwrap(), json!(7));
        assert_eq!(parse_setting("mcp", "false").unwrap(), json!(false));
        assert_eq!(
            parse_setting("providers", "claude, openai").unwrap(),
            json!(["claude", "openai"])
        );
        assert_eq!(
            parse_setting("system_prompt", "be exact").unwrap(),
            json!("be exact")
        );
    }

    #[test]
    fn a_rejected_send_is_terminal_even_though_it_remains_queued() {
        let event = Event::failure("crash", "nothing is running");
        let snapshot = Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            level: 5,
            providers: Vec::new(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: "/tmp".into(),
            seq: 2,
            in_turn: false,
            queued: 1,
        };

        assert!(failed_attempt(Some(&event), Some(&snapshot)));
        assert!(!settled(Some(&snapshot)));
    }
}
