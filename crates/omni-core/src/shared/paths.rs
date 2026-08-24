//! Every path omni owns, in one place.
//!
//! Set `OMNI_HOME` to relocate the whole tree (tests do exactly this).

use std::path::PathBuf;
use std::sync::RwLock;

/// Set once at startup, by the daemon or by a test. Beats the environment so a
/// process can pin its home rather than re-reading a variable anyone could
/// change underneath it.
static PINNED: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Root of omni's state.
pub fn home() -> PathBuf {
    if let Some(path) = PINNED.read().unwrap_or_else(|p| p.into_inner()).clone() {
        return path;
    }
    match std::env::var_os("OMNI_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => dirs_home().join(".omni"),
    }
}

/// Point omni at a different tree. The daemon does this once; tests do it a lot.
pub fn set_home(path: Option<PathBuf>) {
    *PINNED.write().unwrap_or_else(|p| p.into_inner()) = path;
}

pub fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

pub fn sessions() -> PathBuf {
    home().join("sessions")
}

/// The source of truth: full cross-provider history for one omni session.
pub fn session_file(session_id: &str) -> PathBuf {
    sessions().join(format!("{session_id}.jsonl"))
}

/// Which native session each provider holds for this omni session, and how far it is synced.
pub fn meta_file(session_id: &str) -> PathBuf {
    sessions().join(format!("{session_id}.meta.json"))
}

/// A fake provider home, used to strip a CLI of everything it would otherwise
/// auto-load. One per provider, not one per session: its contents never depend
/// on the conversation, and a shared jail is what lets one codex app-server
/// serve every session at once.
pub fn jail(provider: &str) -> PathBuf {
    home().join("jails").join(provider)
}

pub fn cache() -> PathBuf {
    home().join("cache")
}

/// Where the daemon listens. One per `OMNI_HOME`, so a test daemon and a real
/// one never meet.
pub fn socket() -> PathBuf {
    home().join("omnid.sock")
}

/// Who is listening, so a second daemon knows to stand down.
pub fn daemon_lock() -> PathBuf {
    home().join("omnid.pid")
}

pub fn log_file() -> PathBuf {
    home().join("omnid.log")
}

pub fn ensure(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}
