//! What a provider has to be able to do.
//!
//! Two things per provider, deliberately kept apart:
//!
//! [`Account`]
//!   Global, session-free: is the CLI here, are we logged in, how much quota is
//!   left, how do we log in.
//!
//! [`Runner`]
//!   One live CLI process driving one session. It runs turns and reports what
//!   happened as [`Event`] values. It does not own history, session identity,
//!   or model choice — omni does.
//!
//! Adapters stay dumb on purpose. Everything portable lives above them.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::events::{CRASH, Event};

pub const AUTHENTICATED: &str = "authenticated";
pub const UNAUTHENTICATED: &str = "unauthenticated";

/// Where a runner's events go. The conductor hands one of these to every
/// runner it builds, and everything the model does arrives through it.
pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

/// Everything a runner needs to know that is not the conversation itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub append_system_prompt: String,
    /// Both default to on. A provider's own subagents and MCP servers make the
    /// same run mean different things on different machines, so omni starts
    /// from the quiet end and you opt back in with `enable_subagents()`.
    #[serde(default = "on")]
    pub disable_subagents: bool,
    #[serde(default = "on")]
    pub disable_mcp: bool,
    #[serde(default)]
    pub cwd: String,
}

fn on() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: String::new(),
            effort: String::new(),
            system_prompt: String::new(),
            append_system_prompt: String::new(),
            disable_subagents: true,
            disable_mcp: true,
            cwd: here(),
        }
    }
}

impl Config {
    /// Providers resolve symlinks before they name anything after the working
    /// directory — on macOS /var and /tmp are links, and an unresolved path
    /// makes Claude's session file land somewhere omni will never look again.
    pub fn at(mut self, cwd: &str) -> Self {
        self.cwd = real(cwd);
        self
    }
}

pub fn real(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

fn here() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".into())
}

/// Installed-ness, auth and quota for one provider. No session involved.
pub trait Account: Send + Sync {
    /// Dynamic, not `&'static`: a test double is registered under whatever
    /// name the caller picked, and it has to be a provider like any other.
    fn name(&self) -> &str;
    /// The executable to look for on `PATH`.
    fn cli(&self) -> &str;

    fn installed(&self) -> bool {
        which(self.cli()).is_some()
    }

    /// Ask the CLI whether it is signed in. Implemented per provider.
    fn probe(&self) -> String;

    /// Begin a login. Returns the URL the human has to open.
    fn start_auth(&self) -> String {
        format!("omni cannot drive a {} login; run it yourself", self.cli())
    }

    /// Complete a login with the code or redirect URL. Returns the auth status.
    fn finish_auth(&self, _code: &str) -> String {
        self.probe()
    }

    /// `{"5h": {"used": 0.24, "reset": iso}, "7d": {...}}` or `"unauthenticated"`.
    ///
    /// `used` is a fraction (`0.24` is 24%) and `reset` is an RFC3339 UTC
    /// string whatever the provider natively answers in. Either may be `null`:
    /// some plans report no windows at all, and "nobody said" is a different
    /// thing from "you have spent nothing".
    fn limits(&self) -> Value {
        json!({"5h": blank(), "7d": blank()})
    }
}

pub fn blank() -> Value {
    json!({"used": null, "reset": null})
}

/// The first match on `PATH`, the way a shell would find it.
pub fn which(program: &str) -> Option<std::path::PathBuf> {
    if program.contains('/') {
        let path = std::path::PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| std::path::Path::new(dir).join(program))
        .find(|path| path.is_file())
}

/// One provider CLI, driving one session.
///
/// Lifecycle: [`start`](Runner::start) (resuming `native_id` if given,
/// replaying `history` if not empty) → any number of [`send`](Runner::send) →
/// [`stop`](Runner::stop). Everything the model does comes back through the
/// [`Emit`] the runner was built with, ending each turn with an `END` event.
pub trait Runner: Send {
    fn name(&self) -> &str;

    /// This provider's own id for the conversation, once it is up.
    fn native_id(&self) -> String;

    /// Bring the CLI up.
    ///
    /// `native_id` is this provider's own session to resume, if it has one.
    /// `history` is the part of the omni log this provider has not seen and
    /// must be seeded with — empty when we are simply continuing.
    fn start(&mut self, native_id: &str, history: &[Event]) -> Result<(), String>;

    /// Has the history handed to [`start`](Runner::start) actually reached the
    /// provider?
    ///
    /// `false` means omni must not mark this provider as caught up yet — a
    /// runner replaced before it delivers would otherwise skip that history
    /// forever.
    fn seeded(&self) -> bool {
        true
    }

    /// Hand a user message to the CLI, starting or joining a turn.
    fn send(&mut self, text: &str) -> Result<(), String>;

    /// Change model or effort in place, between turns.
    ///
    /// Return `true` if the running process took it. Returning `false` (the
    /// default) makes omni restart the provider instead, which is always
    /// correct but costs re-reading the conversation.
    fn retune(&mut self, _model: &str, _effort: &str) -> bool {
        false
    }

    /// Tell a *running* provider about history it missed, without a restart.
    ///
    /// This is what makes a parked runner worth keeping. Codex can be told
    /// (`thread/inject_items`) and agy can (its seed rides in on the next
    /// message); Claude cannot, because its history is a file it read once at
    /// launch. The default therefore only accepts the case everyone can do —
    /// having missed nothing at all.
    fn catch_up(&mut self, history: &[Event]) -> bool {
        history.is_empty()
    }

    /// Ask the current turn to stop. Best effort.
    fn interrupt(&mut self) {}

    fn stop(&mut self);

    fn alive(&self) -> bool;

    /// Can this runner be put aside, still running, and picked up later?
    ///
    /// A runner that is mid-turn or already dead cannot; everything else can,
    /// and that is what makes coming back to a provider free.
    fn parkable(&self) -> bool {
        self.alive()
    }
}

/// The CLI is gone. If omni did not ask for that, it is a crash.
pub fn exited(name: &str, cli: &str, code: i32) -> Event {
    Event::failure(CRASH, format!("{cli} exited with {code}")).from(name)
}

/// How long to give a CLI to shut down before taking the group out.
pub const GRACE: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omni_starts_from_the_quiet_end() {
        let config = Config::default();
        assert!(config.disable_subagents && config.disable_mcp);
    }

    #[test]
    fn a_working_directory_is_resolved_before_anything_is_named_after_it() {
        let config = Config::default().at("/tmp");
        assert_eq!(config.cwd, real("/tmp"));
        assert!(!config.cwd.is_empty());
    }

    #[test]
    fn a_path_that_does_not_exist_is_kept_rather_than_blanked() {
        assert_eq!(Config::default().at("/no/such/place").cwd, "/no/such/place");
    }

    #[test]
    fn which_finds_what_a_shell_would() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-program-99").is_none());
        assert!(which("/bin/sh").is_some());
    }
}
