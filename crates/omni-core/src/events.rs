//! The event vocabulary.
//!
//! Everything omni has to say arrives as an [`Event`]. The same objects are
//! streamed to every attached client, and appended to the session file — so the
//! session file *is* the event log, and there is only ever one schema to learn.
//!
//! Reasoning is deliberately contentless: a `THINKING` event says the model is
//! thinking, never what it thought. Provider reasoning is encrypted or signed
//! and cannot be replayed into another provider, so omni does not carry it.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::shared::clock;

/// What happened. Which payload fields are populated depends on this.
///
/// | type | carries |
/// |---|---|
/// | `START` | `text` — the user message that opened this turn |
/// | `TEXT` | `text` — one completed assistant message block |
/// | `THINKING` | nothing; the model is reasoning (never the content) |
/// | `TOOL_CALL` | `tool`, `args`, `id` |
/// | `TOOL_RESULT` | `tool`, `id`, `result`, `ok` |
/// | `END` | `extra` — stop reason and usage, if the provider says |
/// | `INJECTED` | `text` — a message that landed mid-turn |
/// | `ERROR` | `error`, `kind` |
/// | `SWITCH_PROVIDER` | `provider` (the new one), `extra["from"]` |
/// | `NEW_SESSION` | `session`, `extra["native"]` |
/// | `CONFIG` | `text` — what changed, `extra` — the new value |
pub mod kind {
    pub const START: &str = "start";
    pub const TEXT: &str = "text";
    pub const THINKING: &str = "thinking";
    pub const TOOL_CALL: &str = "tool.call";
    pub const TOOL_RESULT: &str = "tool.result";
    pub const END: &str = "end";
    pub const INJECTED: &str = "injected";
    pub const ERROR: &str = "error";
    pub const SWITCH_PROVIDER: &str = "switch_provider";
    pub const NEW_SESSION: &str = "new_session";
    pub const CONFIG: &str = "config";
}

/// Error classifications used by [`Event::kind`].
pub const AUTH: &str = "auth";
pub const LIMIT: &str = "limit";
pub const UNAVAILABLE: &str = "unavailable";
pub const CRASH: &str = "crash";

/// Event types that carry conversation content, i.e. the ones replayed into a
/// provider when a session is seeded. Everything else is bookkeeping.
pub const HISTORY_TYPES: &[&str] = &[
    kind::START,
    kind::INJECTED,
    kind::TEXT,
    kind::THINKING,
    kind::TOOL_CALL,
    kind::TOOL_RESULT,
];

/// One thing that happened.
///
/// Three fields are on every event whatever its type. `session` is the omni
/// session it belongs to, `at` is when it happened, and `seq` is its position
/// in that session's log — a number that only goes up, and never repeats,
/// across every provider the conversation has passed through.
///
/// `seq` is how omni knows what a provider still has to be told: the meta file
/// records the last one each provider saw, so coming back to one replays
/// exactly the events recorded since, and nothing twice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub session: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub provider: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub tool: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub ok: bool,
    /// Which sort of failure, when `type` is `error`: one of `auth` / `limit` /
    /// `unavailable` / `crash` from the model or its CLI, `stderr` for CLI
    /// chatter, `omni` when the engine itself failed.
    #[serde(default, rename = "kind", skip_serializing_if = "str::is_empty")]
    pub fault: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub error: String,
    #[serde(default = "clock::now")]
    pub at: String,
    #[serde(default = "unplaced", skip_serializing_if = "is_unplaced")]
    pub seq: i64,
    /// The catch-all. For `CONFIG` events `text` says what changed and this
    /// says what to: `launch`, `retune`, `reseed`, `stop`, `provider_removed`,
    /// `unsupported`, `approximated`, or the name of whatever call was made.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}
fn is_yes(ok: &bool) -> bool {
    *ok
}
fn unplaced() -> i64 {
    -1
}
fn is_unplaced(seq: &i64) -> bool {
    *seq < 0
}

impl Event {
    pub fn new(kind: &str) -> Self {
        Event {
            kind: kind.into(),
            session: String::new(),
            provider: String::new(),
            model: String::new(),
            text: String::new(),
            tool: String::new(),
            id: String::new(),
            args: Map::new(),
            result: Value::Null,
            ok: true,
            fault: String::new(),
            error: String::new(),
            at: clock::now(),
            seq: -1,
            extra: Map::new(),
        }
    }

    /// A failure, in omni's vocabulary. `fault` says which sort.
    pub fn failure(fault: &str, error: impl Into<String>) -> Self {
        Event {
            ok: false,
            fault: fault.into(),
            error: error.into(),
            ..Event::new(kind::ERROR)
        }
    }

    /// A settings change or a piece of bookkeeping. `what` names it.
    pub fn config(what: &str) -> Self {
        Event {
            text: what.into(),
            ..Event::new(kind::CONFIG)
        }
    }

    pub fn from(mut self, provider: &str) -> Self {
        self.provider = provider.into();
        self
    }

    pub fn about(mut self, model: &str) -> Self {
        self.model = model.into();
        self
    }

    pub fn saying(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// One more field on `extra`, so building an event stays a single expression.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    pub fn extras(mut self, extra: Map<String, Value>) -> Self {
        self.extra = extra;
        self
    }

    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }

    /// Does this carry conversation, i.e. would a provider need to be told it?
    pub fn is_history(&self) -> bool {
        HISTORY_TYPES.contains(&self.kind.as_str())
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Which kind of failure this is, in omni's vocabulary.
///
/// Every provider words its failures differently; omni only cares whether you
/// need to log in, wait, retry, or look at a stack trace.
pub fn classify(text: &str) -> &'static str {
    let lowered = text.to_lowercase();
    let faults: [(&'static str, &[&str]); 3] = [
        (AUTH, &["auth", "unauthorized", "401", "login", "sign in"]),
        (LIMIT, &["rate", "limit", "quota", "429"]),
        (
            UNAVAILABLE,
            &[
                "overload",
                "unavailable",
                "disconnect",
                "timeout",
                "404",
                "503",
                "502",
                "model_not_found",
                "model not found",
                "invalid model",
            ],
        ),
    ];
    for (fault, words) in faults {
        if words.iter().any(|word| lowered.contains(word)) {
            return fault;
        }
    }
    CRASH
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_left_out_of_the_session_file() {
        let written = Event::new(kind::THINKING).to_value();
        let keys: Vec<&str> = written
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["at", "type"], "{written}");
    }

    #[test]
    fn ok_is_written_only_when_it_is_false() {
        assert!(Event::new(kind::TEXT).to_value().get("ok").is_none());
        assert_eq!(Event::failure(CRASH, "boom").to_value()["ok"], false);
    }

    #[test]
    fn a_round_trip_keeps_every_field() {
        let event = Event::failure(AUTH, "logged out")
            .from("claude")
            .about("some-model")
            .with("left", 2);
        let back: Event = serde_json::from_value(event.to_value()).unwrap();
        assert_eq!(back.fault, AUTH);
        assert_eq!(back.provider, "claude");
        assert_eq!(back.model, "some-model");
        assert_eq!(back.extra["left"], 2);
        assert!(!back.ok);
    }

    #[test]
    fn an_unknown_field_does_not_break_a_reader() {
        let mut raw = Event::new(kind::TEXT).to_value();
        raw["something_from_the_future"] = serde_json::json!(1);
        let back: Event = serde_json::from_value(raw).unwrap();
        assert!(back.is(kind::TEXT));
    }

    #[test]
    fn failures_are_sorted_into_the_four_kinds_that_matter() {
        assert_eq!(classify("OAuth token expired, please sign in"), AUTH);
        assert_eq!(classify("429 Too Many Requests"), LIMIT);
        assert_eq!(classify("upstream 503"), UNAVAILABLE);
        assert_eq!(classify("thread 'main' panicked"), CRASH);
    }

    #[test]
    fn only_conversation_counts_as_history() {
        assert!(Event::new(kind::TEXT).is_history());
        assert!(Event::new(kind::TOOL_CALL).is_history());
        assert!(!Event::new(kind::END).is_history());
        assert!(!Event::config("launch").is_history());
    }
}
