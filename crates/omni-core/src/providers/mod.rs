//! The providers omni speaks, and the lookup everything else goes through.

pub mod base;
pub mod claude;
pub mod google;
pub mod login;
pub mod openai;
pub mod test;

use std::sync::Arc;

pub use base::{Account, Config, Emit, Runner};

use crate::events::Event;
use crate::shared::clock;

/// The providers built into omni. Doubles are added at runtime by
/// [`test::install`] and appear alongside these.
pub const BUILT_IN: &[&str] = &["claude-code-cli", "codex-app-server", "antigravity-cli"];

pub fn names() -> Vec<String> {
    BUILT_IN
        .iter()
        .map(|name| name.to_string())
        .chain(
            test::installed()
                .into_iter()
                .filter(|name| !BUILT_IN.contains(&name.as_str())),
        )
        .collect()
}

/// The shared account handle for a provider — auth, limits, install state.
/// Can this provider change its system prompt on a session it already opened?
///
/// Claude gets `--system-prompt` on a fresh process every launch, and agy
/// re-seeds its opening, so both take a new prompt on the next turn. Codex does
/// not: `baseInstructions` and `developerInstructions` are fixed when a thread
/// is created, and `thread/resume` drops them. Its own app-server says so —
/// "baseInstructions override was provided and ignored while running" — and a
/// thread resumed with eight facts in its prompt will still recite the five it
/// was born with.
///
/// Anything answering false here has to be given a new native session when the
/// prompt changes, or the change is silently lost.
pub fn retunes_instructions(name: &str) -> bool {
    name != openai::NAME
}

pub fn account(name: &str) -> Option<Arc<dyn Account>> {
    match name {
        "claude-code-cli" => Some(Arc::new(claude::Claude)),
        "codex-app-server" => Some(Arc::new(openai::Codex)),
        "antigravity-cli" => Some(Arc::new(google::Antigravity)),
        other if test::is_installed(other) => Some(Arc::new(test::Signed(other.to_string()))),
        _ => None,
    }
}

pub fn runner(
    name: &str,
    session_id: &str,
    config: &Config,
    emit: Emit,
) -> Option<Box<dyn Runner>> {
    match name {
        "claude-code-cli" => Some(Box::new(claude::Runner::new(session_id, config, emit))),
        "codex-app-server" => Some(Box::new(openai::Runner::new(session_id, config, emit))),
        "antigravity-cli" => Some(Box::new(google::Runner::new(session_id, config, emit))),
        other if test::is_installed(other) => {
            Some(Box::new(test::Double::new(other, session_id, config, emit)))
        }
        _ => None,
    }
}

/// Installed *and* logged in — the only providers omni will route to.
///
/// Every answer costs a CLI invocation, so a "yes" is trusted for a minute and
/// a "no" only briefly: a network blip during a probe would otherwise quietly
/// drop a provider off the dial for the rest of the hour.
///
/// The probes run side by side. They are independent, and one CLI being slow to
/// answer is no reason for the other two to wait — asking three at once is the
/// difference between a second and a minute on a cold daemon.
pub fn available(limit_to: Option<&[String]>) -> Vec<String> {
    let asking: Vec<String> = names()
        .into_iter()
        .filter(|name| limit_to.is_none_or(|allowed| allowed.contains(name)))
        .filter(|name| account(name).is_some_and(|account| account.installed()))
        .collect();
    let probes: Vec<_> = asking
        .into_iter()
        .map(|name| std::thread::spawn(move || (auth_status(&name) == base::AUTHENTICATED, name)))
        .collect();
    probes
        .into_iter()
        .filter_map(|probe| probe.join().ok())
        .filter(|(signed_in, _)| *signed_in)
        .map(|(_, name)| name)
        .collect()
}

/// Seconds to trust a "yes" for.
///
/// Long, because the daemon is asked constantly and a login rarely vanishes —
/// and when one does, the turn that fails says so and clears this, which is a
/// better signal than any poll.
const TTL: f64 = 600.0;
/// Seconds to trust a "no" for. Short, so logging in is picked up at once.
const DOUBT: f64 = 10.0;

static MEMO: std::sync::RwLock<Option<std::collections::BTreeMap<String, (String, f64)>>> =
    std::sync::RwLock::new(None);

/// `"authenticated"` or `"unauthenticated"`, remembered briefly.
pub fn auth_status(name: &str) -> String {
    if let Some(remembered) = recall(name) {
        return remembered;
    }
    let status = account(name).map_or_else(|| base::UNAUTHENTICATED.to_string(), |a| a.probe());
    MEMO.write()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(Default::default)
        .insert(name.to_string(), (status.clone(), clock::epoch()));
    status
}

fn recall(name: &str) -> Option<String> {
    let memo = MEMO.read().unwrap_or_else(|p| p.into_inner());
    let (status, checked) = memo.as_ref()?.get(name)?;
    let window = if status == base::AUTHENTICATED {
        TTL
    } else {
        DOUBT
    };
    (clock::epoch() - checked < window).then(|| status.clone())
}

/// Ask the CLI again next time — after a login, or after losing one.
pub fn forget(name: &str) {
    if let Some(memo) = MEMO.write().unwrap_or_else(|p| p.into_inner()).as_mut() {
        memo.remove(name);
    }
}

pub fn forget_everything() {
    *MEMO.write().unwrap_or_else(|p| p.into_inner()) = None;
}

/// How stale the memo is, in seconds. `None` when nothing has been asked yet.
pub fn probed_ago() -> Option<f64> {
    let memo = MEMO.read().unwrap_or_else(|p| p.into_inner());
    let oldest = memo
        .as_ref()?
        .values()
        .map(|(_, at)| *at)
        .fold(f64::INFINITY, f64::min);
    oldest.is_finite().then(|| clock::epoch() - oldest)
}

/// A provider saying something about itself, rather than about a turn.
pub fn notice(
    provider: &str,
    what: &str,
    extra: serde_json::Map<String, serde_json::Value>,
) -> Event {
    Event::config(what).from(provider).extras(extra)
}
