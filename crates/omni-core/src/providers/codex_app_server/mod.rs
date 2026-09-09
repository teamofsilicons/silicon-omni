//! OpenAI, through `codex app-server`.
//!
//! Not `codex exec`: the app server is a long-lived JSON-RPC peer that can
//! start threads, stream a turn, and — crucially — accept history it never
//! lived through via `thread/inject_items`. That is what makes Codex a
//! first-class destination when a conversation moves.
//!
//! One server, many threads. Every notification that belongs to a turn carries
//! a `threadId`, so a single warm app-server hosts every omni session at once
//! and joining it costs one call instead of a process launch. [`server`] owns
//! that sharing; [`Runner`] just rents a thread from it.
//!
//! Codex reads every setting from one folder, so omni gives it an almost empty
//! one (see [`jail`]) and it has nothing to auto-load.

mod account;
pub mod appserver;
pub mod jail;
mod runner;
pub mod server;
mod stream;

pub use account::Codex;
pub use runner::Runner;
pub use stream::Stream;

pub const NAME: &str = "codex-app-server";
pub const CLI: &str = "codex";
