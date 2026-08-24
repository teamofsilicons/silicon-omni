//! Google, through Antigravity's `agy`.
//!
//! The awkward one. There is no flag for subagents, no flag for MCP, no system
//! prompt flag, and no way to seed a conversation — and the fake-home trick
//! that works for codex fails here because agy's login is tied to the real
//! home. So omni does what it can, says what it cannot, and never pretends.

mod account;
mod runner;
mod stream;

pub use account::Antigravity;
pub use runner::Runner;
pub use stream::Stream;

pub const NAME: &str = "google";
pub const CLI: &str = "agy";
