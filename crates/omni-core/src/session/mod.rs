//! Persistent, cross-provider sessions.
//!
//! `~/.omni/sessions/{id}.jsonl` is the source of truth for a conversation. It
//! outlives any single provider, and it is what a provider gets seeded from
//! when the conversation moves.

mod meta;
mod store;

pub use meta::Meta;
pub use store::Store;
