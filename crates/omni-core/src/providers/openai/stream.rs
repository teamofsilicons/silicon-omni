//! Codex app-server notifications, turned into omni events.
//!
//! Items are a tagged union that grows with every release, so only the four
//! kinds omni has an opinion about are named here — an assistant message,
//! reasoning, the user's own echo, and a shell command. Everything else,
//! present and future, is reported as a tool call by its own type name. New
//! Codex tools work on the day they ship, without a change here.

use serde_json::{Map, Value, json};

use crate::events::{Event, classify, kind};

const SKIP: &[&str] = &["userMessage", "hookPrompt"];
const SHELL: &str = "commandExecution";
const NOISE: &[&str] = &[
    "id",
    "type",
    "status",
    "aggregatedOutput",
    "exitCode",
    "durationMs",
    "processId",
];

#[derive(Default)]
pub struct Stream {
    pub model: String,
    tokens: Value,
}

impl Stream {
    pub fn new(model: &str) -> Self {
        Stream {
            model: model.to_string(),
            tokens: Value::Null,
        }
    }

    fn event(&self, kind: &str) -> Event {
        Event::new(kind).from(super::NAME).about(&self.model)
    }

    pub fn feed(&mut self, method: &str, params: &Value) -> Vec<Event> {
        let mut events = match method {
            "item/started" => self.started(&params["item"]),
            "item/completed" => self.completed(&params["item"]),
            "turn/completed" => self.finished(&params["turn"]),
            "error" => self.failed(params),
            "thread/tokenUsage/updated" => {
                self.tokens = params.clone();
                Vec::new()
            }
            _ => Vec::new(),
        };
        for event in &mut events {
            for (key, value) in [
                ("thread_id", params.get("threadId")),
                (
                    "turn_id",
                    params.get("turnId").or_else(|| params["turn"].get("id")),
                ),
                ("item_id", params["item"].get("id")),
            ] {
                if let Some(value) = value.filter(|value| !value.is_null()) {
                    event.native.insert(key.into(), value.clone());
                }
            }
        }
        events
    }

    fn started(&self, item: &Value) -> Vec<Event> {
        let sort = item.get("type").and_then(Value::as_str).unwrap_or("");
        if SKIP.contains(&sort) || sort == "agentMessage" {
            return Vec::new();
        }
        if sort == "reasoning" {
            return vec![self.event(kind::THINKING)];
        }
        let mut event = self.event(kind::TOOL_CALL);
        event.tool = tool_name(item);
        event.id = item.get("id").and_then(Value::as_str).unwrap_or("").into();
        event.args = arguments(item);
        vec![event]
    }

    fn completed(&self, item: &Value) -> Vec<Event> {
        let sort = item.get("type").and_then(Value::as_str).unwrap_or("");
        if SKIP.contains(&sort) || sort == "reasoning" {
            return Vec::new();
        }
        if sort == "agentMessage" {
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            if text.is_empty() {
                return Vec::new();
            }
            return vec![
                self.event(kind::TEXT)
                    .saying(text)
                    .with("phase", item.get("phase").cloned().unwrap_or(Value::Null)),
            ];
        }
        let mut event = self.event(kind::TOOL_RESULT);
        event.tool = tool_name(item);
        event.id = item.get("id").and_then(Value::as_str).unwrap_or("").into();
        event.result = outcome(item);
        // Nobody saying how it exited is not the same as it exiting badly.
        let exited_well = match item.get("exitCode") {
            None | Some(Value::Null) => true,
            Some(code) => code.as_i64() == Some(0),
        };
        event.ok = exited_well && item.get("status").and_then(Value::as_str) != Some("failed");
        vec![event]
    }

    fn finished(&self, turn: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        if turn.get("status").and_then(Value::as_str) == Some("failed") {
            let error = turn.get("error").cloned().unwrap_or(Value::Null);
            let said = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("turn failed");
            let info = error
                .get("codexErrorInfo")
                .map(Value::to_string)
                .unwrap_or_default();
            let mut event = Event::failure(classify(&format!("{said} {info}")), said)
                .from(super::NAME)
                .about(&self.model);
            event.extra = error.as_object().cloned().unwrap_or_default();
            events.push(event);
        }
        events.push(
            self.event(kind::END)
                .with("status", turn.get("status").cloned().unwrap_or(Value::Null))
                .with("ms", turn.get("durationMs").cloned().unwrap_or(Value::Null))
                .with("usage", self.tokens.clone()),
        );
        events
    }

    /// A mid-turn error. Retryable ones do not end the turn.
    fn failed(&self, params: &Value) -> Vec<Event> {
        let said = params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("codex error");
        let info = params
            .get("codexErrorInfo")
            .map(Value::to_string)
            .unwrap_or_default();
        vec![
            Event::failure(classify(&format!("{said} {info}")), said)
                .from(super::NAME)
                .about(&self.model)
                .with(
                    "willRetry",
                    params.get("willRetry").cloned().unwrap_or(Value::Null),
                ),
        ]
    }
}

fn tool_name(item: &Value) -> String {
    match item.get("type").and_then(Value::as_str) {
        Some(SHELL) => "shell".into(),
        Some(other) => other.into(),
        None => "tool".into(),
    }
}

fn arguments(item: &Value) -> Map<String, Value> {
    if item.get("type").and_then(Value::as_str) == Some(SHELL) {
        let command = item.get("command").cloned().unwrap_or(json!(""));
        return json!({"command": command})
            .as_object()
            .cloned()
            .unwrap_or_default();
    }
    without(item, NOISE)
}

fn outcome(item: &Value) -> Value {
    if item.get("type").and_then(Value::as_str) == Some(SHELL) {
        return item.get("aggregatedOutput").cloned().unwrap_or(Value::Null);
    }
    Value::Object(without(item, &["id", "type"]))
}

fn without(item: &Value, drop: &[&str]) -> Map<String, Value> {
    item.as_object()
        .map(|map| {
            map.iter()
                .filter(|(key, _)| !drop.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_is_noted_once_and_never_carried() {
        let mut stream = Stream::default();
        let started = stream.feed(
            "item/started",
            &json!({"item": {"type": "reasoning", "id": "r"}}),
        );
        assert_eq!(started.len(), 1);
        assert!(started[0].is(kind::THINKING) && started[0].text.is_empty());
        assert!(
            stream
                .feed(
                    "item/completed",
                    &json!({"item": {"type": "reasoning", "id": "r"}})
                )
                .is_empty()
        );
    }

    #[test]
    fn a_shell_command_reads_as_one() {
        let mut stream = Stream::default();
        let call = stream.feed(
            "item/started",
            &json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "item": {"type": SHELL, "id": "c1", "command": "ls -l"}
            }),
        );
        assert_eq!(call[0].tool, "shell");
        assert_eq!(call[0].args["command"], "ls -l");
        assert_eq!(call[0].native["thread_id"], "thread-1");
        assert_eq!(call[0].native["turn_id"], "turn-1");
        assert_eq!(call[0].native["item_id"], "c1");
        let done = stream.feed(
            "item/completed",
            &json!({
                "item": {"type": SHELL, "id": "c1", "aggregatedOutput": "a\nb", "exitCode": 0}
            }),
        );
        assert_eq!(done[0].result, json!("a\nb"));
        assert!(done[0].ok);
    }

    #[test]
    fn a_tool_codex_ships_tomorrow_still_reports_as_a_tool() {
        let mut stream = Stream::default();
        let call = stream.feed(
            "item/started",
            &json!({"item": {"type": "somethingNew", "id": "x", "target": "a"}}),
        );
        assert_eq!(call[0].tool, "somethingNew");
        assert_eq!(
            call[0].args["target"], "a",
            "its own fields become the arguments"
        );
        assert!(
            !call[0].args.contains_key("id"),
            "bookkeeping is not an argument"
        );
    }

    #[test]
    fn a_nonzero_exit_is_not_ok() {
        let mut stream = Stream::default();
        let done = stream.feed(
            "item/completed",
            &json!({
                "item": {"type": SHELL, "id": "c", "aggregatedOutput": "no", "exitCode": 1}
            }),
        );
        assert!(!done[0].ok);
    }

    #[test]
    fn the_users_own_echo_is_not_replayed_as_the_models() {
        let mut stream = Stream::default();
        assert!(
            stream
                .feed(
                    "item/completed",
                    &json!({"item": {"type": "userMessage", "text": "hi"}})
                )
                .is_empty()
        );
    }

    #[test]
    fn a_turn_always_ends_and_a_failed_one_says_why_first() {
        let mut stream = Stream::default();
        let events = stream.feed(
            "turn/completed",
            &json!({
                "turn": {"status": "failed", "error": {"message": "401 unauthorized"}}
            }),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].fault, crate::events::AUTH);
        assert!(events[1].is(kind::END));
    }

    #[test]
    fn a_mid_turn_error_does_not_end_the_turn() {
        let mut stream = Stream::default();
        let events = stream.feed(
            "error",
            &json!({"message": "stream disconnected", "willRetry": true}),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].fault, crate::events::UNAVAILABLE);
        assert_eq!(events[0].extra["willRetry"], true);
    }

    #[test]
    fn usage_reaches_the_end_of_the_turn() {
        let mut stream = Stream::default();
        stream.feed("thread/tokenUsage/updated", &json!({"total": 42}));
        let events = stream.feed("turn/completed", &json!({"turn": {"status": "completed"}}));
        assert_eq!(events[0].extra["usage"]["total"], 42);
    }
}
