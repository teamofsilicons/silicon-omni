//! silicon omni — one engine for Claude Code, Codex and Antigravity.
//!
//! This crate is the whole of omni's behaviour: the event vocabulary, the
//! session log, the provider adapters, the 0-10 intelligence dial, and the
//! conductor that ties them together. It knows nothing about sockets or
//! clients — [`omni-daemon`] wraps it in one, and every language binding talks
//! to that.
//!
//! The rule the whole design hangs off: **nothing changes mid turn**.
//! Intelligence, providers, prompts and session swaps are recorded when you ask
//! for them and applied at the next turn boundary.

pub mod chat;
pub mod events;
pub mod intelligence;
pub mod providers;
pub mod session;
pub mod shared;
pub mod translate;
pub mod wire;

/// Scaffolding for testing code that sits on top of omni. Small on purpose:
/// a private `~/.omni` and the turn-taking that makes one safe.
pub mod testing;

pub use chat::{Change, Chat, Handle, Snapshot, Wake};
pub use events::Event;
