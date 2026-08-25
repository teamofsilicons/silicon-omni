//! `silicon-omni` / `so` — thin terminal clients for `omnid`.

use std::fmt;
use std::io::{self, BufRead, IsTerminal, Write};
use std::time::Duration;

use serde_json::{Value, json};
use silicon_omni::{Client, Event, Frame, OpenOptions, Request, Session, Snapshot, event_type};

const HELP: &str = r#"silicon-omni (so) — one conversation across Claude, Codex, and Antigravity

Usage:
  silicon-omni chat [OPTIONS] SESSION [MESSAGE...]    interactive or one-turn chat
  silicon-omni send [OPTIONS] SESSION MESSAGE...      send and stream until settled
  silicon-omni logs [OPTIONS] SESSION                 replay/follow a session
  silicon-omni history [--since SEQ] [--json] SESSION read persisted events once
  silicon-omni status SESSION                         show a live session snapshot
  silicon-omni sessions                               list live sessions
  silicon-omni stop SESSION                           stop a live session
  silicon-omni set SESSION SETTING VALUE              queue a setting change
  silicon-omni providers [PROVIDER...]                list available providers
  silicon-omni dial [PROVIDER...]                     show the 0–10 intelligence dial
  silicon-omni account PROVIDER ACTION [CODE]         inspect or authenticate an account
  silicon-omni daemon start|status|stop               manage the persistent daemon
  silicon-omni ping                                   show daemon information
  silicon-omni request OP [JSON]                      make a low-level protocol call

Every command is also available through the short `so` executable.

Stream options:
  --since SEQ         first event sequence to replay (send/chat default: -1)
  --providers A,B     limit providers when opening the session
  --intelligence 0..10
                      set intelligence as part of opening
  --json              emit Events as JSON lines
  --frames            emit raw transport Frames as JSON lines

Compatibility aliases:
  attach = logs, events = history, --from = --since, --level = --intelligence

Account actions:
  status (default), installed, limits, start-auth, finish-auth CODE, forget

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

impl From<silicon_omni::Error> for CliError {
    fn from(error: silicon_omni::Error) -> Self {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Output {
    Human,
    Json,
    Frames,
}

pub fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("silicon-omni: {error}");
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
        "version" | "-V" | "--version" => {
            println!("silicon-omni {}", env!("CARGO_PKG_VERSION"))
        }
        "ping" => print_value(&serde_json::to_value(Client::connect()?.ping()?)?),
        "daemon" => daemon(args)?,
        "providers" => providers(args)?,
        "dial" => dial(args)?,
        "sessions" => sessions(no_args(command, args)?)?,
        "status" => status(one_arg(command, args)?)?,
        "history" | "events" => history(args)?,
        "logs" | "attach" => logs(args)?,
        "send" => stream(args, false)?,
        "chat" => stream(args, true)?,
        "stop" => stop(one_arg(command, args)?)?,
        "set" => set(args)?,
        "account" => account(args)?,
        "request" => request(args)?,
        other => {
            return Err(CliError(format!(
                "unknown command {other:?}; try `silicon-omni help`"
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
                if silicon_omni::wait_until_stopped(Duration::from_secs(90)) {
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
    println!("intelligence  provider  model  effort");
    for (intelligence, rung) in table.iter().rev() {
        println!(
            "{intelligence:>12}  {:<8}  {}  {}",
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
    println!("session\tstatus\tprovider\tmodel\tintelligence\tlisteners");
    for live in sessions {
        let state = live.snapshot;
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            state.session,
            state.status,
            state.provider,
            state.model,
            state.intelligence,
            live.listeners
        );
    }
    Ok(())
}

fn status(session: String) -> Result<()> {
    let (snapshot, listeners) = Client::connect()?.status(&session)?;
    print_value(&json!({"snapshot": public_snapshot(&snapshot), "listeners": listeners}));
    Ok(())
}

fn history(mut args: Vec<String>) -> Result<()> {
    let output = output(&mut args, false)?;
    let since = take_aliased_i64(&mut args, "--since", "--from")?.unwrap_or(0);
    let session = one_arg("history", args)?;
    for event in Client::connect()?.events(&session, since)? {
        print_event(&event, output)?;
    }
    Ok(())
}

fn logs(mut args: Vec<String>) -> Result<()> {
    let output = output(&mut args, true)?;
    let since = take_aliased_i64(&mut args, "--since", "--from")?.unwrap_or(0);
    let providers = take_providers(&mut args)?;
    let session_id = one_arg("logs", args)?;
    let client = Client::connect()?;
    let mut options = OpenOptions::new(&session_id).from_seq(since);
    if let Some(providers) = providers {
        options = options.providers(providers);
    }
    let mut session = client.open(options)?;
    loop {
        let frame = session.recv()?;
        let gone = frame.stream == "gone";
        print_frame(&frame, output)?;
        if gone {
            break;
        }
    }
    let _ = session.detach();
    Ok(())
}

fn stream(mut args: Vec<String>, interactive: bool) -> Result<()> {
    let output = output(&mut args, true)?;
    let since = take_aliased_i64(&mut args, "--since", "--from")?.unwrap_or(-1);
    let intelligence = take_aliased_i64(&mut args, "--intelligence", "--level")?;
    if intelligence.is_some_and(|intelligence| !(0..=10).contains(&intelligence)) {
        return Err(CliError("--intelligence must be between 0 and 10".into()));
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
    let mut options = OpenOptions::new(session_id).from_seq(since);
    if let Some(providers) = providers {
        options = options.providers(providers);
    }
    if let Some(intelligence) = intelligence {
        options = options.setting("intelligence", intelligence);
    }
    let mut session = client.open(options)?;

    // `open` may replay before its reply. Show those frames now so an old END
    // cannot be mistaken for the boundary of the message about to be sent.
    for _ in 0..session.opened().replayed {
        print_frame(&session.recv()?, output)?;
    }

    if !initial.is_empty() {
        send_and_wait(&session, &initial, output)?;
    }
    if interactive {
        repl(&session, output)?;
    }
    session.detach()?;
    Ok(())
}

fn repl(session: &Session, output: Output) -> Result<()> {
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
        send_and_wait(session, text, output)?;
    }
    Ok(())
}

fn send_and_wait(session: &Session, text: &str, output: Output) -> Result<()> {
    if !session.send(text)? {
        return Err(CliError("daemon did not accept the message".into()));
    }
    let mut acknowledged = false;
    let mut failure = None;
    loop {
        let frame = session.recv()?;
        let event = frame.event.as_ref();
        if event.is_some_and(|event| {
            matches!(
                event.event_type.as_str(),
                event_type::START | event_type::INJECTED
            ) && event.text == text
        }) {
            acknowledged = true;
        }
        if let Some(error) = event.filter(|event| {
            event.event_type == event_type::ERROR
                && event.kind != "stderr"
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
        print_frame(&frame, output)?;
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
    event.is_some_and(|event| event.event_type == event_type::ERROR)
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
    let what = canonical_setting(&args.remove(0));
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
        "start-auth" | "login" | "start" => ("start_auth", None),
        "finish-auth" | "finish" => {
            if args.is_empty() {
                return Err(CliError(
                    "account finish-auth needs a code or redirect URL".into(),
                ));
            }
            ("finish_auth", Some(args.join(" ")))
        }
        "forget" => ("forget", None),
        other => {
            return Err(CliError(format!(
                "unknown account action {other:?}; use status, installed, limits, start-auth, finish-auth, or forget"
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

fn print_frame(frame: &Frame, output: Output) -> Result<()> {
    match output {
        Output::Frames => println!("{}", serde_json::to_string(frame)?),
        Output::Json => {
            if let Some(event) = frame.event.as_ref() {
                print_event(event, Output::Json)?;
            }
        }
        Output::Human => {
            if let Some(event) = frame.event.as_ref() {
                print_event(event, Output::Human)?;
            } else if frame.stream == "gone" {
                eprintln!("[session stopped]");
            }
        }
    }
    Ok(())
}

fn print_event(event: &Event, output: Output) -> Result<()> {
    if output != Output::Human {
        println!("{}", serde_json::to_string(&public_event(event))?);
        return Ok(());
    }
    match event.event_type.as_str() {
        event_type::TEXT => {
            println!("{}", event.text);
            io::stdout().flush()?;
        }
        event_type::START => eprintln!("> {}", event.text),
        event_type::INJECTED => eprintln!(">> {}", event.text),
        event_type::THINKING => eprintln!("[thinking]"),
        event_type::TOOL_CALL => eprintln!(
            "[tool {} {}]",
            event.tool,
            serde_json::to_string(&event.args)?
        ),
        event_type::TOOL_RESULT => eprintln!(
            "[tool {} {}]",
            event.tool,
            if event.ok { "ok" } else { "failed" }
        ),
        event_type::ERROR => eprintln!(
            "[error{}] {}",
            if event.kind.is_empty() {
                String::new()
            } else {
                format!("/{}", event.kind)
            },
            event.error
        ),
        event_type::SWITCH_PROVIDER => eprintln!("[provider → {}]", event.provider),
        event_type::NEW_SESSION => eprintln!("[new native session: {}]", event.provider),
        event_type::CONFIG => eprintln!("[config: {}]", event.text),
        event_type::END => {}
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

fn public_snapshot(snapshot: &Snapshot) -> Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or(Value::Null);
    rename_key(&mut value, "level", "intelligence");
    value
}

fn public_event(event: &Event) -> Value {
    let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
    if let Some(extra) = value.get_mut("extra") {
        rename_key(extra, "level", "intelligence");
    }
    value
}

fn rename_key(value: &mut Value, old: &str, new: &str) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if let Some(value) = object.remove(old) {
        object.entry(new).or_insert(value);
    }
}

fn canonical_setting(what: &str) -> String {
    match what {
        "level" => "intelligence".into(),
        other => other.into(),
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

fn take_aliased_i64(
    args: &mut Vec<String>,
    canonical: &str,
    compatibility: &str,
) -> Result<Option<i64>> {
    let canonical_value = take_i64(args, canonical)?;
    let compatibility_value = take_i64(args, compatibility)?;
    if canonical_value.is_some() && compatibility_value.is_some() {
        return Err(CliError(format!(
            "use {canonical} only; {compatibility} is its compatibility alias"
        )));
    }
    Ok(canonical_value.or(compatibility_value))
}

fn output(args: &mut Vec<String>, frames_allowed: bool) -> Result<Output> {
    let json = take_flag(args, "--json");
    let frames = take_flag(args, "--frames");
    if json && frames {
        return Err(CliError("use either --json or --frames, not both".into()));
    }
    if frames && !frames_allowed {
        return Err(CliError(
            "--frames is only available on streaming commands".into(),
        ));
    }
    Ok(if frames {
        Output::Frames
    } else if json {
        Output::Json
    } else {
        Output::Human
    })
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
        let mut args = vec![
            "demo".into(),
            "--since".into(),
            "12".into(),
            "--json".into(),
        ];
        assert!(take_flag(&mut args, "--json"));
        assert_eq!(
            take_aliased_i64(&mut args, "--since", "--from").unwrap(),
            Some(12)
        );
        assert_eq!(args, vec!["demo"]);
    }

    #[test]
    fn compatibility_options_work_but_cannot_be_mixed_with_the_canonical_name() {
        let mut old = vec!["--from".into(), "12".into()];
        assert_eq!(
            take_aliased_i64(&mut old, "--since", "--from").unwrap(),
            Some(12)
        );

        let mut both = vec![
            "--intelligence".into(),
            "7".into(),
            "--level".into(),
            "6".into(),
        ];
        assert!(
            take_aliased_i64(&mut both, "--intelligence", "--level")
                .unwrap_err()
                .to_string()
                .contains("compatibility alias")
        );
    }

    #[test]
    fn setting_values_follow_the_wire_types() {
        assert_eq!(canonical_setting("level"), "intelligence");
        assert_eq!(parse_setting("intelligence", "7").unwrap(), json!(7));
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
    fn public_json_names_intelligence_and_keeps_event_payloads_exact() {
        let snapshot = Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            intelligence: 5,
            providers: Vec::new(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: "/tmp".into(),
            seq: 2,
            in_turn: false,
            queued: 0,
        };
        let public = public_snapshot(&snapshot);
        assert_eq!(public["intelligence"], 5);
        assert!(public.get("level").is_none());

        let mut event = Event::new(Event::TOOL_CALL);
        event.args.insert("level".into(), json!("provider-native"));
        event.extra.insert("level".into(), json!(5));
        let public = public_event(&event);
        assert_eq!(public["args"]["level"], "provider-native");
        assert_eq!(public["extra"]["intelligence"], 5);
        assert!(public["extra"].get("level").is_none());
    }

    #[test]
    fn help_leads_with_the_shared_vocabulary() {
        assert!(HELP.contains("history [--since SEQ]"));
        assert!(HELP.contains("logs [OPTIONS]"));
        assert!(HELP.contains("--intelligence 0..10"));
        assert!(HELP.contains("attach = logs, events = history"));
    }

    #[test]
    fn a_rejected_send_is_terminal_even_though_it_remains_queued() {
        let event = Event::failure("crash", "nothing is running");
        let snapshot = Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            intelligence: 5,
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
