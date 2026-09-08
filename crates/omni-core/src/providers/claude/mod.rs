//! Claude Code.
//!
//! Flags do all the work here: subagents, MCP, slash commands and memory files
//! can all be switched off from the command line, and `--session-id` lets omni
//! pick the session uuid up front. Seeding is a file write — see [`session`].

mod account;
mod control;
mod runner;
pub mod session;
mod stream;

pub use account::Claude;
pub use runner::Runner;
pub use stream::Stream;

pub const NAME: &str = "claude-code-cli";
pub const CLI: &str = "claude-code-cli";
