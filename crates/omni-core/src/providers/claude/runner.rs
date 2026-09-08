//! Running Claude Code for one omni session.
//!
//! One long-lived `claude -p` process handles every turn: messages go in as
//! NDJSON on stdin, events come back on stdout, and the process stays up
//! between turns. A message written while a turn is in flight is picked up by
//! Claude at the next safe point, which is exactly the injection behaviour omni
//! promises.
//!
//! Seeding is a file write before launch (see [`session`](super::session)), so
//! switching to Claude costs nothing but the disk.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use crate::events::Event;
use crate::providers::base::{self, Config, Delivery, Emit, Runner as RunnerTrait};
use crate::shared::proc::{LineProcess, Spawn};
use crate::translate::transcript;

use super::stream::Stream;
use super::{control, session};

/// Always on: memory files would make the same run mean different things on
/// different machines. Subagents and MCP are off by default but can be opted
/// back into per session; these cannot.
const QUIET_ENV: &[(&str, &str)] = &[
    ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1"),
    ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1"),
    ("CLAUDE_CODE_DISABLE_ORG_MEMORY", "1"),
];

/// Whoever is waiting on a control response, by request id.
type Waiting = Arc<Mutex<Vec<(String, mpsc::Sender<Value>)>>>;

pub struct Runner {
    config: Config,
    emit: Emit,
    stream: Arc<Mutex<Stream>>,
    proc: Option<LineProcess>,
    native: String,
    waiting: Waiting,
    tickets: AtomicU64,
    stopping: Arc<std::sync::atomic::AtomicBool>,
}

impl Runner {
    /// Claude keys everything off its own session uuid, so omni's session id is
    /// not needed here — the signature stays uniform across adapters.
    pub fn new(_session_id: &str, config: &Config, emit: Emit) -> Self {
        Runner {
            config: config.clone(),
            emit,
            stream: Arc::new(Mutex::new(Stream::new(&config.model))),
            proc: None,
            native: String::new(),
            waiting: Arc::default(),
            tickets: AtomicU64::new(1),
            stopping: Arc::default(),
        }
    }

    fn argv(&self, resume: bool) -> Vec<String> {
        let mut argv: Vec<String> = [
            "claude-code-cli",
            "-p",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--verbose",
            "--replay-user-messages",
            "--dangerously-skip-permissions",
            "--disable-slash-commands",
        ]
        .iter()
        .map(|part| part.to_string())
        .collect();
        argv.push(if resume {
            "--resume".into()
        } else {
            "--session-id".into()
        });
        argv.push(self.native.clone());
        for (flag, value) in [
            ("--model", &self.config.model),
            ("--effort", &self.config.effort),
            ("--system-prompt", &self.config.system_prompt),
            ("--append-system-prompt", &self.config.append_system_prompt),
        ] {
            if !value.is_empty() {
                argv.push(flag.into());
                argv.push(value.clone());
            }
        }
        if self.config.disable_subagents {
            argv.extend(["--disallowedTools".to_string(), "Agent(*)".to_string()]);
        }
        if self.config.disable_mcp {
            argv.extend(["--strict-mcp-config", "--setting-sources", ""].map(str::to_string));
        }
        argv
    }

    /// A control request omni only believes once the CLI says it worked.
    fn ask(&self, subtype: &str, fields: Value, timeout: Duration) -> bool {
        let Some(proc) = &self.proc else { return false };
        let ticket = self.tickets.fetch_add(1, Ordering::SeqCst);
        let request_id = format!("omni-{subtype}-{ticket}");
        let (tx, rx) = mpsc::channel();
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((request_id.clone(), tx));
        if !proc.send_line(&control::line(subtype, &request_id, fields)) {
            self.forget(&request_id);
            return false;
        }
        let answered = rx.recv_timeout(timeout).ok();
        self.forget(&request_id);
        answered.is_some_and(|response| control::succeeded(&response))
    }

    fn forget(&self, request_id: &str) {
        self.waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(id, _)| id != request_id);
    }
}

impl RunnerTrait for Runner {
    fn name(&self) -> &str {
        super::NAME
    }

    fn native_id(&self) -> String {
        self.native.clone()
    }

    fn start(&mut self, native_id: &str, history: &[Event]) -> Result<(), String> {
        // Resume is scoped to the working directory: a session file that is not
        // under this cwd's slug does not exist as far as claude is concerned.
        // Reporting a different id back is how omni learns to reseed in full.
        let lost =
            !native_id.is_empty() && !session::session_path(&self.config.cwd, native_id).exists();
        self.native = match (lost, native_id) {
            (false, given) if !given.is_empty() => given.to_string(),
            _ => uuid::Uuid::new_v4().to_string(),
        };
        let turns = if lost {
            Vec::new()
        } else {
            transcript(history)
        };
        if !turns.is_empty() {
            session::seed(&self.config.cwd, &self.native, &turns, &self.config.model)
                .map_err(|err| format!("could not seed claude: {err}"))?;
        }
        let known = session::session_path(&self.config.cwd, &self.native).exists();

        let stream = self.stream.clone();
        let waiting = self.waiting.clone();
        let emit = self.emit.clone();
        let heard = emit.clone();
        let stopping = self.stopping.clone();
        let mut spawn = Spawn::new(self.argv(known))
            .cwd(self.config.cwd.clone())
            .on_line(move |line| {
                if let Some((request_id, response)) = control::answer(line) {
                    let mut slots = waiting.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(at) = slots.iter().position(|(id, _)| *id == request_id) {
                        let (_, tx) = slots.remove(at);
                        let _ = tx.send(response);
                        return;
                    }
                }
                for event in stream.lock().unwrap_or_else(|p| p.into_inner()).feed(line) {
                    heard(event);
                }
            })
            .on_stderr({
                let emit = emit.clone();
                move |text| {
                    // Claude warns on stderr. Worth surfacing, never worth a
                    // teardown: only an exit reports a crash, because a line
                    // that merely says "error" may be a deprecation notice or a
                    // tool's own output.
                    if text.to_lowercase().starts_with("error") {
                        emit(Event::failure("stderr", text).from(super::NAME));
                    }
                }
            })
            .on_exit(move |code| {
                if !stopping.load(Ordering::SeqCst) {
                    emit(base::exited(super::NAME, super::CLI, code));
                }
            });
        for (key, value) in QUIET_ENV {
            spawn = spawn.env(key, *value);
        }
        if self.config.disable_subagents {
            spawn = spawn.env("CLAUDE_CODE_DISABLE_WORKFLOWS", "1");
        }
        self.proc = Some(
            spawn
                .start()
                .map_err(|err| format!("could not start claude: {err}"))?,
        );
        Ok(())
    }

    fn send(&mut self, text: &str) -> Result<Delivery, String> {
        let Some(proc) = &self.proc else {
            return Err("claude would not take the message".into());
        };
        // Hold this lock across the pipe write. The stdout reader uses the
        // same lock, so even an immediate replay echo sees its FIFO entry.
        let mut stream = self.stream.lock().unwrap_or_else(|p| p.into_inner());
        stream.expect_user(text);
        if proc.send_line(&session::user_line(text)) {
            Ok(Delivery::Echoed)
        } else {
            stream.forget_last_user();
            Err("claude would not take the message".into())
        }
    }

    /// Swap model or effort over the control channel — no restart, no re-read.
    ///
    /// A rejected request means omni restarts instead, rather than reporting a
    /// model the CLI is not actually using.
    fn retune(&mut self, model: &str, effort: &str) -> bool {
        if !self.alive() {
            return false;
        }
        if !model.is_empty()
            && model != self.config.model
            && !self.ask(
                "set_model",
                json!({"model": model}),
                Duration::from_secs(15),
            )
        {
            return false;
        }
        if effort != self.config.effort {
            // An empty effort means "back to the default", and the flag layer
            // is replaced wholesale — so it has to be sent, not skipped.
            let settings = if effort.is_empty() {
                json!({})
            } else {
                json!({"effortLevel": effort})
            };
            if !self.ask(
                "apply_flag_settings",
                json!({"settings": settings}),
                Duration::from_secs(15),
            ) {
                return false;
            }
        }
        self.config.model = model.to_string();
        self.config.effort = effort.to_string();
        self.stream.lock().unwrap_or_else(|p| p.into_inner()).model = model.to_string();
        true
    }

    fn interrupt(&mut self) {
        if let Some(proc) = &self.proc {
            proc.send_line(&control::line("interrupt", "omni-interrupt", json!({})));
        }
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        if let Some(proc) = self.proc.take() {
            proc.stop(base::GRACE);
        }
    }

    fn alive(&self) -> bool {
        self.proc.as_ref().is_some_and(LineProcess::alive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(config: Config) -> Runner {
        Runner::new("s", &config, Arc::new(|_| {}))
    }

    fn argv_of(config: Config) -> Vec<String> {
        let mut runner = runner(config);
        runner.native = "uuid-1".into();
        runner.argv(false)
    }

    #[test]
    fn the_quiet_defaults_are_on_the_command_line() {
        let argv = argv_of(Config::default());
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--disallowedTools", "Agent(*)"])
        );
        assert!(argv.iter().any(|part| part == "--strict-mcp-config"));
        assert!(
            argv.iter()
                .any(|part| part == "--dangerously-skip-permissions")
        );
        assert!(argv.iter().any(|part| part == "--replay-user-messages"));
    }

    #[test]
    fn opting_back_in_takes_the_flags_away() {
        let argv = argv_of(Config {
            disable_subagents: false,
            disable_mcp: false,
            ..Default::default()
        });
        assert!(!argv.iter().any(|part| part == "--disallowedTools"));
        assert!(!argv.iter().any(|part| part == "--strict-mcp-config"));
    }

    #[test]
    fn a_fresh_session_is_named_and_a_known_one_is_resumed() {
        let mut runner = runner(Config::default());
        runner.native = "uuid-1".into();
        assert!(
            runner
                .argv(false)
                .windows(2)
                .any(|p| p == ["--session-id", "uuid-1"])
        );
        assert!(
            runner
                .argv(true)
                .windows(2)
                .any(|p| p == ["--resume", "uuid-1"])
        );
    }

    #[test]
    fn an_empty_setting_is_left_off_rather_than_passed_empty() {
        let argv = argv_of(Config::default());
        assert!(!argv.iter().any(|part| part == "--model"), "{argv:?}");
        assert!(!argv.iter().any(|part| part == "--system-prompt"));
    }
}
