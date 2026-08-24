//! Running Codex for one omni session.
//!
//! A thread is Codex's session. omni rents one from a warm app-server (see
//! [`server`](super::server)), or resumes the one this omni session already
//! owns, and seeds anything it missed with `thread/inject_items` — the stable
//! path, verified to make the model answer from history it never saw live.
//!
//! A message that arrives mid-turn is steered into the running turn rather than
//! queued behind it, which is the same promise omni makes everywhere else.
//!
//! Stopping does not stop the server. A thread is cheap and a server is not, so
//! putting a runner down only unhooks it — coming back is one call.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::events::Event;
use crate::providers::base::{Config, Delivery, Emit, Runner as RunnerTrait};
use crate::translate::{Turn, transcript};

use super::server::{self, Shared};

/// Never a switch: `AGENTS.md` and friends would make the same run mean
/// different things depending on whose directory it started in.
const NO_MEMORIES: &[&str] = &["-c", "project_doc_max_bytes=0"];
const NO_SUBAGENTS: &[&str] = &[
    "--disable",
    "apps",
    "--disable",
    "plugins",
    "-c",
    "agents.enabled=false",
];

/// omni turns as Codex `ResponseItem`s.
fn items(turns: &[Turn]) -> Value {
    Value::Array(
        turns
            .iter()
            .map(|turn| {
                let sort = if turn.role == "user" {
                    "input_text"
                } else {
                    "output_text"
                };
                json!({
                    "type": "message",
                    "role": turn.role,
                    "content": [{"type": sort, "text": turn.text}],
                })
            })
            .collect(),
    )
}

/// MCP is already gone with the jail; this is everything else.
///
/// Project docs go regardless. Subagents are the only part you can ask for
/// back, and asking must not quietly bring the memories with them.
pub fn flags(config: &Config) -> Vec<String> {
    let subagents = if config.disable_subagents {
        NO_SUBAGENTS
    } else {
        &[]
    };
    subagents
        .iter()
        .chain(NO_MEMORIES)
        .map(|part| part.to_string())
        .collect()
}

pub struct Runner {
    config: Config,
    emit: Emit,
    shared: Option<Arc<Shared>>,
    thread: String,
}

impl Runner {
    pub fn new(_session_id: &str, config: &Config, emit: Emit) -> Self {
        Runner {
            config: config.clone(),
            emit,
            shared: None,
            thread: String::new(),
        }
    }

    /// MCP is not a switch here, so say so rather than let a caller believe it.
    ///
    /// CODEX_HOME is redirected whether or not it was asked for — the jail *is*
    /// how omni isolates codex. A chat that opts back into MCP still does not
    /// get it, and quietly not getting it is the worst of the options.
    fn announce(&self) {
        if self.config.disable_mcp {
            return;
        }
        (self.emit)(
            Event::config("unsupported")
                .from(super::NAME)
                .with("ignored", json!(["enable_mcp"]))
                .with(
                    "why",
                    "codex always runs in a jailed CODEX_HOME, so MCP cannot load",
                ),
        );
    }

    /// Pick the thread back up. If Codex has lost it, start a clean one.
    fn resume(&self, shared: &Shared, native_id: &str) -> Result<String, String> {
        let mut body = server::settings(&self.config);
        body["threadId"] = json!(native_id);
        match shared.call("thread/resume", body, Duration::from_secs(60)) {
            Ok(back) => Ok(back["thread"]["id"]
                .as_str()
                .unwrap_or(native_id)
                .to_string()),
            // Losing Codex's native thread is recoverable: `Chat::launch`
            // notices the replacement id and records the reseed. Emitting a
            // fatal CRASH here races with that replacement and can tear the
            // healthy new thread down after it was adopted.
            Err(_) => self.open(shared),
        }
    }

    fn open(&self, shared: &Shared) -> Result<String, String> {
        let started = shared
            .call(
                "thread/start",
                server::settings(&self.config),
                Duration::from_secs(60),
            )
            .map_err(|err| err.to_string())?;
        Ok(started["thread"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    /// Land a message inside the running turn.
    ///
    /// The turn can finish between reading its id and codex hearing about it,
    /// so a refusal means open a new turn instead of losing the message.
    fn steer(&self, shared: &Shared, text: &str, turn: &str) -> bool {
        shared
            .try_call(
                "turn/steer",
                json!({
                    "threadId": self.thread,
                    "expectedTurnId": turn,
                    "input": [{"type": "text", "text": text}],
                }),
                Duration::from_secs(30),
            )
            .is_some()
    }
}

impl RunnerTrait for Runner {
    fn name(&self) -> &str {
        super::NAME
    }

    fn native_id(&self) -> String {
        self.thread.clone()
    }

    fn generation(&self) -> Option<u64> {
        self.shared.as_ref().map(|shared| shared.generation())
    }

    fn start(&mut self, native_id: &str, history: &[Event]) -> Result<(), String> {
        self.announce();
        let shared = Shared::get(&flags(&self.config)).map_err(|err| err.to_string())?;
        self.thread = if native_id.is_empty() {
            self.open(&shared)?
        } else {
            self.resume(&shared, native_id)?
        };
        if self.thread.is_empty() {
            return Err("codex started a thread with no id".into());
        }
        if self.config.disable_subagents {
            shared.silence_skills(&self.config.cwd);
        }
        let seed = transcript(history);
        if !seed.is_empty() {
            shared
                .call(
                    "thread/inject_items",
                    json!({"threadId": self.thread, "items": items(&seed)}),
                    Duration::from_secs(120),
                )
                .map_err(|err| err.to_string())?;
        }
        if !shared.attach(&self.thread, &self.config.model, self.emit.clone()) {
            return Err("codex app-server exited while attaching the thread".into());
        }
        self.shared = Some(shared);
        Ok(())
    }

    fn send(&mut self, text: &str) -> Result<Delivery, String> {
        let Some(shared) = &self.shared else {
            return Err("codex is not running".into());
        };
        let turn = shared.turn_of(&self.thread);
        if !turn.is_empty() && self.steer(shared, text, &turn) {
            return Ok(Delivery::Immediate);
        }
        let mut body = json!({
            "threadId": self.thread,
            "summary": "none",
            "input": [{"type": "text", "text": text}],
        });
        for (key, value) in [
            ("model", &self.config.model),
            ("effort", &self.config.effort),
        ] {
            if !value.is_empty() {
                body[key] = json!(value);
            }
        }
        shared
            .call("turn/start", body, Duration::from_secs(60))
            .map(|_| Delivery::NextTurn)
            .map_err(|err| err.to_string())
    }

    /// A live thread accepts history the same way a fresh one does, so coming
    /// back to a warm Codex costs one call and no restart.
    fn catch_up(&mut self, history: &[Event]) -> bool {
        let Some(shared) = &self.shared else {
            return false;
        };
        let seed = transcript(history);
        if seed.is_empty() {
            return self.alive();
        }
        shared
            .call(
                "thread/inject_items",
                json!({"threadId": self.thread, "items": items(&seed)}),
                Duration::from_secs(120),
            )
            .is_ok()
    }

    /// Codex takes model and effort per turn, so this costs nothing at all.
    fn retune(&mut self, model: &str, effort: &str) -> bool {
        self.config.model = model.to_string();
        self.config.effort = effort.to_string();
        if let Some(shared) = &self.shared {
            shared.retune(&self.thread, model);
        }
        self.alive()
    }

    fn interrupt(&mut self) {
        let Some(shared) = &self.shared else { return };
        let turn = shared.turn_of(&self.thread);
        if !turn.is_empty() {
            shared.try_call(
                "turn/interrupt",
                json!({"threadId": self.thread, "turnId": turn}),
                Duration::from_secs(10),
            );
        }
    }

    /// Unhook, but leave the server up: that is what makes coming back cheap.
    fn stop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.detach(&self.thread);
        }
    }

    fn alive(&self) -> bool {
        self.shared.as_ref().is_some_and(|shared| shared.alive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memories_go_whether_or_not_the_subagents_do() {
        let quiet = flags(&Config::default());
        let loud = flags(&Config {
            disable_subagents: false,
            ..Config::default()
        });
        assert!(quiet.iter().any(|f| f == "agents.enabled=false"));
        assert!(!loud.iter().any(|f| f == "agents.enabled=false"));
        for set in [&quiet, &loud] {
            assert!(
                set.iter().any(|f| f == "project_doc_max_bytes=0"),
                "{set:?}"
            );
        }
    }

    #[test]
    fn a_flag_set_is_what_a_shared_server_is_keyed_on() {
        assert_ne!(
            flags(&Config::default()),
            flags(&Config {
                disable_subagents: false,
                ..Config::default()
            }),
            "two isolation settings must not share one server"
        );
    }

    #[test]
    fn history_becomes_the_items_codex_accepts() {
        let turns = [
            Turn {
                role: "user",
                text: "hi".into(),
            },
            Turn {
                role: "assistant",
                text: "hello".into(),
            },
        ];
        let sent = items(&turns);
        assert_eq!(sent[0]["content"][0]["type"], "input_text");
        assert_eq!(sent[1]["content"][0]["type"], "output_text");
        assert_eq!(sent[1]["role"], "assistant");
    }
}
