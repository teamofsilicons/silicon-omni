//! Claude's `--output-format stream-json`, turned into omni events.
//!
//! One line in, zero or more events out. The parser holds just enough state to
//! name a tool result after the call it belongs to, and to remember the session
//! id and model Claude reports at the top of every turn.
//!
//! Thinking blocks arrive with a signature and are encrypted; omni notes that
//! the model thought and throws the content away.

use std::collections::{BTreeMap, VecDeque};

use serde_json::{Value, json};

use crate::events::{Event, classify, event_type};
use crate::providers::base;

const THINKING_BLOCKS: &[&str] = &["thinking", "redacted_thinking"];

#[derive(Default)]
pub struct Stream {
    tools: BTreeMap<String, String>,
    expected: VecDeque<String>,
    turn_open: bool,
    pub session_id: String,
    pub model: String,
}

impl Stream {
    pub fn new(model: &str) -> Self {
        Stream {
            model: model.to_string(),
            ..Default::default()
        }
    }

    fn event(&self, kind: &str) -> Event {
        let mut event = Event::new(kind).from(super::NAME).about(&self.model);
        if !self.session_id.is_empty() {
            event
                .native
                .insert("session_id".into(), json!(self.session_id));
        }
        event
    }

    /// Register a line before it reaches the pipe. The runner holds the stream
    /// lock across the write, so a very fast replay echo cannot beat this.
    pub fn expect_user(&mut self, text: &str) {
        self.expected.push_back(text.to_string());
    }

    pub fn forget_last_user(&mut self) {
        self.expected.pop_back();
    }

    pub fn feed(&mut self, line: &str) -> Vec<Event> {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match data.get("type").and_then(Value::as_str) {
            Some("system") => self.system(&data),
            Some("assistant") => self.assistant(&data),
            Some("user") if replay_echo(&data) => self.replayed(&data),
            Some("user") => self.results(&data),
            Some("rate_limit_event") => self.rate_limited(&data),
            Some("result") => self.finished(&data),
            _ => Vec::new(),
        }
    }

    /// `init` opens every turn and carries the session id and model.
    fn system(&mut self, data: &Value) -> Vec<Event> {
        if data.get("subtype").and_then(Value::as_str) == Some("init") {
            if let Some(id) = data.get("session_id").and_then(Value::as_str) {
                self.session_id = id.to_string();
            }
            if let Some(model) = data.get("model").and_then(Value::as_str) {
                self.model = model.to_string();
            }
        }
        Vec::new()
    }

    fn assistant(&mut self, data: &Value) -> Vec<Event> {
        let message = &data["message"];
        let message_id = message.get("id").and_then(Value::as_str).unwrap_or("");
        if let Some(model) = message.get("model").and_then(Value::as_str) {
            self.model = model.to_string();
        }
        let content = message["content"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if api_error_message(data, message) {
            let error = content
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let error = if error.is_empty() {
                data.get("error")
                    .and_then(Value::as_str)
                    .or_else(|| message.get("error").and_then(Value::as_str))
                    .filter(|text| !text.is_empty())
                    .unwrap_or("Claude returned an API error")
                    .to_string()
            } else {
                error
            };
            let classification = format!(
                "{} {} {}",
                error,
                data.get("error").and_then(Value::as_str).unwrap_or(""),
                message.get("error").and_then(Value::as_str).unwrap_or("")
            );
            let mut event = Event::failure(classify(&classification), error)
                .from(super::NAME)
                .about(&self.model);
            if !message_id.is_empty() {
                event.native.insert("message_id".into(), json!(message_id));
            }
            if !self.session_id.is_empty() {
                event
                    .native
                    .insert("session_id".into(), json!(self.session_id));
            }
            return vec![event];
        }
        let mut events = Vec::new();
        for block in content {
            let block_kind = block.get("type").and_then(Value::as_str).unwrap_or("");
            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
            if block_kind == "text" {
                if !text.is_empty() {
                    events.push(self.event(event_type::TEXT).saying(text));
                }
            } else if THINKING_BLOCKS.contains(&block_kind) {
                events.push(self.event(event_type::THINKING));
            } else if block_kind == "tool_use" {
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.tools.insert(id.clone(), name.clone());
                let mut event = self.event(event_type::TOOL_CALL);
                event.tool = name;
                event.id = id;
                event.args = block["input"].as_object().cloned().unwrap_or_default();
                if !event.id.is_empty() {
                    event.native.insert("tool_use_id".into(), json!(event.id));
                }
                events.push(event);
            } else {
                // Claude can add content block types without changing the
                // stream envelope. Keep their structured payload in the one
                // protocol-compatible place a portable assistant message has:
                // Event.text. Known encrypted thinking stays excluded above.
                let text = flatten_content(Some(block));
                if !text.is_empty() {
                    events.push(self.event(event_type::TEXT).saying(text));
                }
            }
        }
        if !message_id.is_empty() {
            for event in &mut events {
                event.native.insert("message_id".into(), json!(message_id));
            }
        }
        events
    }

    /// `--replay-user-messages` is Claude's delivery acknowledgement. Only an
    /// echo matching the FIFO of lines omni actually wrote can advance native
    /// turn state; control-channel chatter also arrives as replayed user text.
    fn replayed(&mut self, data: &Value) -> Vec<Event> {
        let text = flatten_content(data["message"].get("content"));
        if self.expected.front().map(String::as_str) != Some(text.as_str()) {
            return Vec::new();
        }
        self.expected.pop_front();
        let opening = if self.turn_open {
            event_type::INJECTED
        } else {
            event_type::START
        };
        self.turn_open = true;
        let mut event = base::confirmed(&text, opening)
            .from(super::NAME)
            .about(&self.model);
        for key in ["session_id", "uuid", "timestamp"] {
            if let Some(value) = data.get(key).filter(|value| !value.is_null()) {
                event.native.insert(key.into(), value.clone());
            }
        }
        if !self.session_id.is_empty() && !event.native.contains_key("session_id") {
            event
                .native
                .insert("session_id".into(), json!(self.session_id));
        }
        vec![event]
    }

    /// A `user` line from Claude is a tool result coming back.
    fn results(&mut self, data: &Value) -> Vec<Event> {
        let Some(content) = data["message"]["content"].as_array() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let call_id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut event = self.event(event_type::TOOL_RESULT);
            event.tool = self.tools.remove(&call_id).unwrap_or_default();
            event.id = call_id;
            if !event.id.is_empty() {
                event.native.insert("tool_use_id".into(), json!(event.id));
            }
            event.result = Value::String(flatten_content(block.get("content")));
            event.ok = !block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            events.push(event);
        }
        events
    }

    fn rate_limited(&mut self, data: &Value) -> Vec<Event> {
        let info = &data["rate_limit_info"];
        if info.get("status").and_then(Value::as_str) != Some("rejected") {
            return Vec::new();
        }
        let sort = info
            .get("rateLimitType")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let mut event = Event::failure(crate::events::LIMIT, format!("rate limited ({sort})"))
            .from(super::NAME)
            .about(&self.model);
        event.extra = info.as_object().cloned().unwrap_or_default();
        vec![event]
    }

    /// `result` closes the turn, successfully or not.
    fn finished(&mut self, data: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        let subtype = data.get("subtype").and_then(Value::as_str).unwrap_or("");
        let result = data.get("result").and_then(Value::as_str).unwrap_or("");
        if data
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || subtype != "success"
        {
            let status = data
                .get("api_error_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            let error = match (result, subtype) {
                ("", "") => "unknown error",
                ("", other) => other,
                (said, _) => said,
            };
            events.push(
                Event::failure(classify(&format!("{subtype} {result} {status}")), error)
                    .from(super::NAME)
                    .about(&self.model),
            );
        }
        events.push(
            self.event(event_type::END)
                .with(
                    "stop_reason",
                    data.get("stop_reason").cloned().unwrap_or(Value::Null),
                )
                .with(
                    "cost_usd",
                    data.get("total_cost_usd").cloned().unwrap_or(Value::Null),
                )
                .with("usage", data.get("usage").cloned().unwrap_or(Value::Null))
                .with(
                    "turns",
                    data.get("num_turns").cloned().unwrap_or(Value::Null),
                ),
        );
        self.turn_open = false;
        events
    }
}

fn replay_echo(data: &Value) -> bool {
    if !data
        .get("isReplay")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let content = &data["message"]["content"];
    let text = flatten_content(Some(content));
    if text.starts_with("<local-command-stdout>") || text == "[Request interrupted by user]" {
        return false;
    }
    !content.as_array().is_some_and(|blocks| {
        blocks
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

fn api_error_message(data: &Value, message: &Value) -> bool {
    [data, message].iter().any(|value| {
        ["is_api_error_message", "isApiErrorMessage"]
            .iter()
            .any(|field| value.get(field).and_then(Value::as_bool).unwrap_or(false))
    })
}

/// Tool output arrives as a string or a list of blocks; omni wants text.
pub fn flatten_content(content: Option<&Value>) -> String {
    match content {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(flatten_block)
            .filter(|block| !block.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        Some(other) => flatten_block(other),
    }
}

fn flatten_block(block: &Value) -> String {
    match block {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Object(map)
            if map.get("type").and_then(Value::as_str) == Some("text")
                || (map.len() == 1 && map.contains_key("text")) =>
        {
            map.get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        }
        Value::Object(_) => block.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed(stream: &mut Stream, value: Value) -> Vec<Event> {
        stream.feed(&value.to_string())
    }

    #[test]
    fn a_line_that_is_not_json_is_ignored_rather_than_fatal() {
        assert!(Stream::default().feed("not json at all").is_empty());
    }

    #[test]
    fn init_names_the_session_and_the_model() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            json!({"type": "system", "subtype": "init", "session_id": "u1", "model": "m"}),
        );
        assert_eq!(
            (stream.session_id.as_str(), stream.model.as_str()),
            ("u1", "m")
        );
    }

    #[test]
    fn thinking_is_noted_and_never_carried() {
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({
                "type": "assistant",
                "message": {"content": [{"type": "thinking", "thinking": "secret", "signature": "x"}]}
            }),
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].is(event_type::THINKING));
        assert!(events[0].text.is_empty(), "the content never leaves claude");
    }

    #[test]
    fn a_synthetic_api_error_is_an_error_not_assistant_text() {
        let mut stream = Stream::new("claude-sonnet");
        let events = feed(
            &mut stream,
            json!({
                "type": "assistant",
                "is_api_error_message": true,
                "message": {
                    "model": "<synthetic>",
                    "content": [{"type": "text", "text": "API Error: 401 unauthorized"}]
                }
            }),
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].is(event_type::ERROR));
        assert_eq!(events[0].kind, crate::events::AUTH);
        assert_eq!(events[0].error, "API Error: 401 unauthorized");
        assert!(events[0].text.is_empty());
    }

    #[test]
    fn a_bad_model_api_error_is_unavailable() {
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({
                "type": "assistant",
                "is_api_error_message": true,
                "error": "model_not_found",
                "message": {
                    "model": "<synthetic>",
                    "content": [{
                        "type": "text",
                        "text": "There's an issue with the selected model"
                    }]
                }
            }),
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].is(event_type::ERROR));
        assert_eq!(events[0].kind, crate::events::UNAVAILABLE);
        assert!(events[0].text.is_empty());
    }

    #[test]
    fn a_tool_result_is_named_after_the_call_it_answers() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            json!({
                "type": "assistant",
                "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}}]}
            }),
        );
        let events = feed(
            &mut stream,
            json!({
                "type": "user",
                "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "a\nb"}]}
            }),
        );
        assert_eq!(events[0].tool, "Bash");
        assert_eq!(events[0].result, json!("a\nb"));
        assert!(events[0].ok);
    }

    #[test]
    fn replay_echoes_confirm_expected_users_in_native_turn_order() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            json!({"type": "system", "subtype": "init", "session_id": "s1", "model": "m"}),
        );

        // Control-channel chatter is also labelled as replayed user text. It
        // neither confirms our line nor opens a native turn.
        stream.expect_user("one");
        assert!(
            feed(
                &mut stream,
                json!({
                    "type": "user", "isReplay": true,
                    "message": {"content": "<local-command-stdout>Set model</local-command-stdout>"}
                })
            )
            .is_empty()
        );

        let first = feed(
            &mut stream,
            json!({
                "type": "user", "isReplay": true, "session_id": "s1",
                "uuid": "u1", "timestamp": "2026-08-24T00:00:00Z",
                "message": {"content": [{"type": "text", "text": "one"}]}
            }),
        );
        assert_eq!(first.len(), 1);
        assert!(first[0].is(base::CONFIRMED));
        assert_eq!(first[0].extra[base::CONFIRMED_AS], event_type::START);
        assert_eq!(first[0].native["session_id"], "s1");
        assert_eq!(first[0].native["uuid"], "u1");

        stream.expect_user("two");
        let injected = feed(
            &mut stream,
            json!({
                "type": "user", "isReplay": true,
                "message": {"content": "two"}
            }),
        );
        assert_eq!(injected[0].extra[base::CONFIRMED_AS], event_type::INJECTED);

        feed(
            &mut stream,
            json!({"type": "result", "subtype": "success", "is_error": false}),
        );
        stream.expect_user("three");
        let next = feed(
            &mut stream,
            json!({
                "type": "user", "isReplay": true,
                "message": {"content": "three"}
            }),
        );
        assert_eq!(next[0].extra[base::CONFIRMED_AS], event_type::START);
    }

    #[test]
    fn an_unexpected_replay_cannot_ack_or_reorder_the_fifo() {
        let mut stream = Stream::default();
        stream.expect_user("same");
        stream.expect_user("same");
        assert!(
            feed(
                &mut stream,
                json!({"type": "user", "isReplay": true, "message": {"content": "other"}})
            )
            .is_empty()
        );
        for opening in [event_type::START, event_type::INJECTED] {
            let events = feed(
                &mut stream,
                json!({"type": "user", "isReplay": true, "message": {"content": "same"}}),
            );
            assert_eq!(events[0].extra[base::CONFIRMED_AS], opening);
        }
        assert!(
            feed(
                &mut stream,
                json!({"type": "user", "isReplay": true, "message": {"content": "same"}}),
            )
            .is_empty(),
            "a third duplicate had no corresponding pipe write"
        );
    }

    #[test]
    fn a_failed_tool_says_so() {
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({
                "type": "user",
                "message": {"content": [{"type": "tool_result", "tool_use_id": "t9", "content": "boom", "is_error": true}]}
            }),
        );
        assert!(!events[0].ok);
    }

    #[test]
    fn a_turn_always_ends_even_when_it_failed() {
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({
                "type": "result", "subtype": "error_during_execution", "is_error": true,
                "result": "Invalid API key · Please run /login"
            }),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, crate::events::AUTH);
        assert!(events[1].is(event_type::END));
    }

    #[test]
    fn a_rejected_rate_limit_is_an_error_and_an_accepted_one_is_not() {
        let mut stream = Stream::default();
        assert!(
            feed(
                &mut stream,
                json!({
                    "type": "rate_limit_event", "rate_limit_info": {"status": "allowed"}
                })
            )
            .is_empty()
        );
        let events = feed(
            &mut stream,
            json!({
                "type": "rate_limit_event",
                "rate_limit_info": {"status": "rejected", "rateLimitType": "five_hour"}
            }),
        );
        assert_eq!(events[0].kind, crate::events::LIMIT);
    }

    #[test]
    fn tool_output_is_text_whatever_shape_it_arrived_in() {
        assert_eq!(flatten_content(Some(&json!("plain"))), "plain");
        assert_eq!(
            flatten_content(Some(
                &json!([{"type": "text", "text": "a"}, {"type": "text", "text": "b"}])
            )),
            "a\nb"
        );
        assert_eq!(flatten_content(None), "");
    }

    #[test]
    fn structured_tool_output_is_serialized_instead_of_disappearing() {
        let structured = json!({
            "type": "image",
            "source": {"type": "base64", "media_type": "image/png", "data": "abc"}
        });
        let flattened = flatten_content(Some(&json!([
            {"type": "text", "text": "before"},
            structured.clone()
        ])));
        let (before, encoded) = flattened.split_once('\n').unwrap();
        assert_eq!(before, "before");
        assert_eq!(serde_json::from_str::<Value>(encoded).unwrap(), structured);
    }

    #[test]
    fn an_unknown_assistant_block_survives_as_textual_json() {
        let block = json!({
            "type": "server_tool_result",
            "status": "complete",
            "text": "a human-readable field must not erase the rest",
            "content": [{"title": "source", "url": "https://example.test"}]
        });
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({"type": "assistant", "message": {"content": [block.clone()]}}),
        );

        assert_eq!(events.len(), 1);
        assert!(events[0].is(event_type::TEXT));
        assert_eq!(
            serde_json::from_str::<Value>(&events[0].text).unwrap(),
            block
        );
        let round_trip: Event = serde_json::from_value(events[0].to_value()).unwrap();
        assert_eq!(round_trip.text, events[0].text);
    }
}
