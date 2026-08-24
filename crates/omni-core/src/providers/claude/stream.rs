//! Claude's `--output-format stream-json`, turned into omni events.
//!
//! One line in, zero or more events out. The parser holds just enough state to
//! name a tool result after the call it belongs to, and to remember the session
//! id and model Claude reports at the top of every turn.
//!
//! Thinking blocks arrive with a signature and are encrypted; omni notes that
//! the model thought and throws the content away.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::events::{Event, classify, kind};

const THINKING_BLOCKS: &[&str] = &["thinking", "redacted_thinking"];

#[derive(Default)]
pub struct Stream {
    tools: BTreeMap<String, String>,
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
        Event::new(kind).from(super::NAME).about(&self.model)
    }

    pub fn feed(&mut self, line: &str) -> Vec<Event> {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match data.get("type").and_then(Value::as_str) {
            Some("system") => self.system(&data),
            Some("assistant") => self.assistant(&data),
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
            return vec![
                Event::failure(classify(&classification), error)
                    .from(super::NAME)
                    .about(&self.model),
            ];
        }
        let mut events = Vec::new();
        for block in content {
            let block_kind = block.get("type").and_then(Value::as_str).unwrap_or("");
            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
            if block_kind == "text" && !text.is_empty() {
                events.push(self.event(kind::TEXT).saying(text));
            } else if THINKING_BLOCKS.contains(&block_kind) {
                events.push(self.event(kind::THINKING));
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
                let mut event = self.event(kind::TOOL_CALL);
                event.tool = name;
                event.id = id;
                event.args = block["input"].as_object().cloned().unwrap_or_default();
                events.push(event);
            }
        }
        events
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
            let mut event = self.event(kind::TOOL_RESULT);
            event.tool = self.tools.remove(&call_id).unwrap_or_default();
            event.id = call_id;
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
            self.event(kind::END)
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
        events
    }
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
            .map(|block| match block {
                Value::Object(map) => map
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        Some(other) => other.to_string(),
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
        assert!(events[0].is(kind::THINKING));
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
        assert!(events[0].is(kind::ERROR));
        assert_eq!(events[0].fault, crate::events::AUTH);
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
        assert!(events[0].is(kind::ERROR));
        assert_eq!(events[0].fault, crate::events::UNAVAILABLE);
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
        assert_eq!(events[0].fault, crate::events::AUTH);
        assert!(events[1].is(kind::END));
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
        assert_eq!(events[0].fault, crate::events::LIMIT);
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
}
