//! Running Antigravity for one omni session.
//!
//! agy cannot be seeded, so history it missed is flattened into text and folded
//! into the front of the next thing the user says — one turn, not two. Its own
//! conversations are resumed with `--conversation`, but an id it no longer
//! recognises is silently replaced with a fresh one, so the id it reports back
//! is always checked against the one that was asked for.
//!
//! Startup is slow (the CLI boots a language server every time), which is why
//! [`start`](RunnerTrait::start) waits for the `init` line before handing back
//! — and why the daemon keeps a stopped agy parked rather than killed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::json;

use crate::events::Event;
use crate::providers::base::{self, Config, Delivery, Emit, Runner as RunnerTrait};
use crate::shared::proc::{LineProcess, Spawn};
use crate::translate::{SEED_HEADER, flatten};

use super::stream::{Stream, user_line};

/// Cold start is ~10s, but a loaded machine can take much longer.
const READY: Duration = Duration::from_secs(120);
const INSTRUCTIONS: &str = "Follow these instructions for the rest of this conversation:";

pub struct Runner {
    config: Config,
    emit: Emit,
    stream: Arc<Mutex<Stream>>,
    proc: Option<LineProcess>,
    native: String,
    /// What agy has to be told before it can carry on. Empty once it has been.
    seed: String,
    stopping: Arc<AtomicBool>,
}

impl Runner {
    pub fn new(_session_id: &str, config: &Config, emit: Emit) -> Self {
        Runner {
            config: config.clone(),
            emit,
            stream: Arc::new(Mutex::new(Stream::new(&config.model))),
            proc: None,
            native: String::new(),
            seed: String::new(),
            stopping: Arc::default(),
        }
    }

    /// Say which of omni's switches agy simply does not have.
    ///
    /// There is no flag for either, and its login is tied to the real home so
    /// the fake-home trick that works for codex is out. Better to say so than
    /// to let a caller believe subagents are off when they are not.
    fn announce(&self) {
        let ignored: Vec<&str> = [
            ("disable_subagents", self.config.disable_subagents),
            ("disable_mcp", self.config.disable_mcp),
        ]
        .iter()
        .filter(|(_, asked)| *asked)
        .map(|(name, _)| *name)
        .collect();
        if ignored.is_empty() {
            return;
        }
        (self.emit)(
            Event::config("unsupported")
                .from(super::NAME)
                .with("ignored", json!(ignored))
                .with("why", "agy has no switch for these"),
        );
    }

    /// What agy has to be told before it can carry on: prompt, then history.
    ///
    /// There is no system prompt flag either, so the prompt goes in as text —
    /// an approximation, and omni says so rather than dropping it.
    fn opening(&self, history: &[Event]) -> String {
        let prompt = [
            self.config.system_prompt.as_str(),
            self.config.append_system_prompt.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        if !prompt.is_empty() {
            (self.emit)(Event::config("approximated").from(super::NAME).with(
                "system_prompt",
                "sent as text; agy has no system prompt flag",
            ));
        }
        [
            if prompt.is_empty() {
                String::new()
            } else {
                format!("{INSTRUCTIONS}\n\n{prompt}")
            },
            flatten(history, SEED_HEADER),
        ]
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n")
    }

    fn argv(&self, native_id: &str) -> Vec<String> {
        let mut argv: Vec<String> = [
            "agy",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--disable-slash-commands",
            "--print-timeout",
            "24h",
            "--dangerously-skip-permissions",
        ]
        .iter()
        .map(|part| part.to_string())
        .collect();
        for (flag, value) in [
            ("--model", &self.config.model),
            ("--effort", &self.config.effort),
        ] {
            if !value.is_empty() {
                argv.push(flag.into());
                argv.push(value.clone());
            }
        }
        if !native_id.is_empty() {
            argv.extend(["--conversation".to_string(), native_id.to_string()]);
        }
        // --print takes a value, so it goes last with an explicit empty one.
        // Anywhere else it silently swallows the flag that follows it.
        argv.extend(["--print".to_string(), String::new()]);
        argv
    }

    fn finish_startup(&mut self) -> Result<(), String> {
        self.native = self
            .stream
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .conversation
            .clone();
        if self.native.is_empty() {
            // A successful start owns a resumable conversation. Leaving a
            // nameless process behind would both leak it and let the conductor
            // advance metadata for a session agy cannot resume.
            self.stop();
            return Err("agy did not report a conversation id during startup".into());
        }
        Ok(())
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
        self.stopping.store(false, Ordering::SeqCst);
        self.announce();
        self.seed = self.opening(history);
        let (ready_tx, ready_rx) = mpsc::channel();
        let stream = self.stream.clone();
        let emit = self.emit.clone();
        let heard = emit.clone();
        let stopping = self.stopping.clone();
        let told = stopping.clone();
        let announced = Arc::new(AtomicBool::new(false));
        let proc = Spawn::new(self.argv(native_id))
            .cwd(self.config.cwd.clone())
            .on_line(move |line| {
                let events = stream.lock().unwrap_or_else(|p| p.into_inner()).feed(line);
                for event in events {
                    heard(event);
                }
                let named = !stream
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .conversation
                    .is_empty();
                if named && !announced.swap(true, Ordering::SeqCst) {
                    let _ = ready_tx.send(());
                }
            })
            .on_stderr({
                let emit = emit.clone();
                move |text| {
                    // Only an exit reports a crash; stderr is just agy talking.
                    if text.to_lowercase().starts_with("error") {
                        emit(Event::failure("stderr", text).from(super::NAME));
                    }
                }
            })
            .on_exit(move |code| {
                if !told.load(Ordering::SeqCst) {
                    emit(base::exited(super::NAME, super::CLI, code));
                }
            })
            .start()
            .map_err(|err| format!("could not start agy: {err}"))?;
        self.proc = Some(proc);
        // Nobody should wait out the cold start on a corpse, so give up early
        // if the process is already gone.
        let deadline = std::time::Instant::now() + READY;
        while std::time::Instant::now() < deadline {
            if ready_rx.recv_timeout(Duration::from_millis(100)).is_ok() {
                break;
            }
            if !self.alive() {
                break;
            }
        }
        self.finish_startup()
    }

    /// agy is only told anything when the next message goes out.
    fn seeded(&self) -> bool {
        self.seed.is_empty()
    }

    /// agy has no seeding call, but it never needed one: history it missed is
    /// text, and text can be folded into whatever is said next.
    fn catch_up(&mut self, history: &[Event]) -> bool {
        let missed = flatten(history, SEED_HEADER);
        if !missed.is_empty() {
            self.seed = match self.seed.is_empty() {
                true => missed,
                false => format!("{}\n\n{missed}", self.seed),
            };
        }
        self.alive()
    }

    fn send(&mut self, text: &str) -> Result<Delivery, String> {
        let Some(proc) = &self.proc else {
            return Err("agy is not running".into());
        };
        let message = if self.seed.is_empty() {
            text.to_string()
        } else {
            format!("{}\n\n---\n\n{text}", std::mem::take(&mut self.seed))
        };
        if proc.send_line(&user_line(&message)) {
            Ok(Delivery::NextTurn)
        } else {
            Err("agy would not take the message".into())
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
    use crate::events::event_type;

    fn runner(config: Config) -> Runner {
        Runner::new("s", &config, Arc::new(|_| {}))
    }

    #[test]
    fn print_is_last_and_carries_its_own_empty_value() {
        let argv = runner(Config::default()).argv("");
        assert_eq!(
            &argv[argv.len() - 2..],
            &["--print".to_string(), String::new()]
        );
    }

    #[test]
    fn a_known_conversation_is_resumed() {
        let argv = runner(Config::default()).argv("c1");
        assert!(argv.windows(2).any(|pair| pair == ["--conversation", "c1"]));
        assert!(
            !runner(Config::default())
                .argv("")
                .iter()
                .any(|f| f == "--conversation")
        );
    }

    #[test]
    fn the_turn_never_times_out() {
        let argv = runner(Config::default()).argv("");
        assert!(
            argv.windows(2)
                .any(|pair| pair == ["--print-timeout", "24h"])
        );
    }

    #[test]
    fn history_and_prompt_ride_in_on_the_next_message() {
        let mut runner = runner(Config {
            system_prompt: "be terse".into(),
            ..Config::default()
        });
        runner.seed = runner.opening(&[Event::new(event_type::START).saying("earlier")]);
        assert!(!runner.seeded(), "nothing has reached agy yet");
        assert!(runner.seed.contains("be terse") && runner.seed.contains("earlier"));
    }

    #[test]
    fn replacement_and_appended_prompts_are_both_sent_in_order() {
        let runner = runner(Config {
            system_prompt: "replacement first".into(),
            append_system_prompt: "append second".into(),
            ..Config::default()
        });
        let opening = runner.opening(&[]);
        assert_eq!(
            opening,
            format!("{INSTRUCTIONS}\n\nreplacement first\n\nappend second")
        );
    }

    #[test]
    fn nothing_to_carry_means_nothing_to_say() {
        let mut runner = runner(Config::default());
        runner.seed = runner.opening(&[]);
        assert!(runner.seeded(), "an empty seed is already delivered");
    }

    #[test]
    fn startup_without_a_conversation_id_fails_and_reaps_agy() {
        let mut runner = runner(Config::default());
        runner.proc = Some(Spawn::new(["cat"]).start().unwrap());
        assert!(runner.alive());

        let err = runner.finish_startup().unwrap_err();
        assert!(err.contains("conversation id"));
        assert!(
            !runner.alive(),
            "the nameless process was stopped and reaped"
        );
        assert!(runner.proc.is_none());
    }
}
