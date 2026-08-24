//! agy's stream-json, turned into omni events.
//!
//! Everything arrives as `step_update` lines keyed by `step_index`: a step goes
//! `ACTIVE` then `DONE`, and assistant text comes as deltas that have to be
//! stitched back together. Short answers sometimes skip `ACTIVE` entirely, so
//! nothing here assumes a step was announced before it finished.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::events::{Event, classify, kind};

#[derive(Default)]
pub struct Stream {
    pub model: String,
    pub conversation: String,
    text: BTreeMap<i64, String>,
    called: BTreeSet<i64>,
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
        if !self.conversation.is_empty() {
            event
                .native
                .insert("conversation_id".into(), json!(self.conversation));
        }
        event
    }

    pub fn feed(&mut self, line: &str) -> Vec<Event> {
        let Ok(data) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        match data.get("event").and_then(Value::as_str) {
            Some("init") => {
                if let Some(id) = data["conversation_id"].as_str() {
                    self.conversation = id.to_string();
                }
                Vec::new()
            }
            Some("step_update") => {
                self.remember_conversation(&data["step_update"]);
                self.step(&data["step_update"])
            }
            Some("result") => {
                self.remember_conversation(&data["result"]);
                self.finished(&data["result"])
            }
            _ => Vec::new(),
        }
    }

    fn remember_conversation(&mut self, value: &Value) {
        if let Some(id) = value.get("conversation_id").and_then(Value::as_str) {
            self.conversation = id.to_string();
        }
    }

    fn step_event(&self, kind: &str, index: i64) -> Event {
        let mut event = self.event(kind);
        if index >= 0 {
            event.native.insert("step_index".into(), json!(index));
        }
        event
    }

    fn step(&mut self, step: &Value) -> Vec<Event> {
        match step.get("step_type").and_then(Value::as_str) {
            Some("agent_response") => self.speech(step),
            Some("tool") => self.tool(step),
            _ => Vec::new(),
        }
    }

    fn speech(&mut self, step: &Value) -> Vec<Event> {
        let index = step["step_index"].as_i64().unwrap_or(-1);
        let delta = step["text_delta"].as_str().unwrap_or("");
        self.text.entry(index).or_default().push_str(delta);
        if step["state"].as_str() == Some("ACTIVE") {
            return Vec::new();
        }
        let mut events = Vec::new();
        if step["usage"]["thinking_tokens"].as_i64().unwrap_or(0) > 0 {
            events.push(self.step_event(kind::THINKING, index));
        }
        let said = self
            .text
            .remove(&index)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !said.is_empty() {
            events.push(self.step_event(kind::TEXT, index).saying(said));
        }
        events
    }

    fn tool(&mut self, step: &Value) -> Vec<Event> {
        let index = step["step_index"].as_i64().unwrap_or(-1);
        let info = &step["tool_info"];
        let name = step["tool_name"]
            .as_str()
            .or_else(|| info["name"].as_str())
            .unwrap_or("tool")
            .to_string();
        let mut events = Vec::new();
        if self.called.insert(index) {
            let mut call = self.step_event(kind::TOOL_CALL, index);
            call.tool = name.clone();
            call.id = index.to_string();
            call.args = info["parameters"].as_object().cloned().unwrap_or_default();
            events.push(call);
        }
        if step["state"].as_str() == Some("ACTIVE") {
            return events;
        }
        self.called.remove(&index);
        let failure = &info["error"];
        let failed = failure.is_object();
        let mut result = self.step_event(kind::TOOL_RESULT, index);
        result.tool = name;
        result.id = index.to_string();
        result.result = if failed {
            failure["message"].clone()
        } else {
            info["output"].clone()
        };
        result.ok = !failed && step["state"].as_str() != Some("ERROR");
        events.push(result);
        events
    }

    fn finished(&mut self, result: &Value) -> Vec<Event> {
        // Step indexes are scoped to one native turn. A failed or interrupted
        // turn may never send DONE for its active steps, and agy reuses the
        // same indexes on the next turn. Keeping either accumulator here would
        // prepend stale text or suppress the next tool call.
        self.text.clear();
        self.called.clear();
        let mut events = Vec::new();
        if result["status"].as_str() == Some("ERROR") {
            let said = result["error"].as_str().unwrap_or("agy turn failed");
            let mut event = Event::failure(classify(said), said)
                .from(super::NAME)
                .about(&self.model);
            if !self.conversation.is_empty() {
                event
                    .native
                    .insert("conversation_id".into(), json!(self.conversation));
            }
            events.push(event);
        }
        events.push(
            self.event(kind::END)
                .with(
                    "status",
                    result.get("status").cloned().unwrap_or(Value::Null),
                )
                .with(
                    "seconds",
                    result
                        .get("duration_seconds")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .with("usage", result.get("usage").cloned().unwrap_or(Value::Null)),
        );
        events
    }
}

/// agy keys its input on `event`, not `type`.
pub fn user_line(text: &str) -> String {
    json!({"event": "user", "message": {"role": "user", "content": text}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(stream: &mut Stream, value: Value) -> Vec<Event> {
        stream.feed(&value.to_string())
    }

    #[test]
    fn init_names_the_conversation() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            json!({"event": "init", "conversation_id": "c1"}),
        );
        assert_eq!(stream.conversation, "c1");
    }

    #[test]
    fn deltas_are_stitched_back_into_one_message() {
        let mut stream = Stream::default();
        for part in ["Hel", "lo ", "there"] {
            assert!(
                feed(
                    &mut stream,
                    json!({
                        "event": "step_update",
                        "step_update": {"step_type": "agent_response", "step_index": 0,
                                        "state": "ACTIVE", "text_delta": part}
                    })
                )
                .is_empty()
            );
        }
        let done = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "agent_response", "step_index": 0, "state": "DONE"}
            }),
        );
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].text, "Hello there");
        assert_eq!(done[0].native["step_index"], 0);
    }

    #[test]
    fn a_short_answer_that_never_said_active_still_arrives() {
        let mut stream = Stream::default();
        let done = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "agent_response", "step_index": 4,
                                "state": "DONE", "text_delta": "yes"}
            }),
        );
        assert_eq!(done[0].text, "yes");
    }

    #[test]
    fn thinking_is_reported_when_it_was_paid_for() {
        let mut stream = Stream::default();
        let done = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "agent_response", "step_index": 0, "state": "DONE",
                                "text_delta": "hi", "usage": {"thinking_tokens": 12}}
            }),
        );
        assert!(done[0].is(kind::THINKING) && done[1].is(kind::TEXT));
    }

    #[test]
    fn a_tool_is_called_once_however_many_updates_it_gets() {
        let mut stream = Stream::default();
        let mut calls = 0;
        for _ in 0..3 {
            calls += feed(
                &mut stream,
                json!({
                    "event": "step_update",
                    "step_update": {"step_type": "tool", "step_index": 1, "state": "ACTIVE",
                                    "tool_name": "GoogleSearch",
                                    "tool_info": {"parameters": {"query": "kites"}}}
                }),
            )
            .iter()
            .filter(|event| event.is(kind::TOOL_CALL))
            .count();
        }
        assert_eq!(calls, 1);
        let done = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "tool", "step_index": 1, "state": "DONE",
                                "tool_name": "GoogleSearch", "tool_info": {"output": "12 results"}}
            }),
        );
        assert!(done[0].is(kind::TOOL_RESULT) && done[0].ok);
        assert_eq!(done[0].result, json!("12 results"));
    }

    #[test]
    fn a_tool_that_errored_says_so() {
        let mut stream = Stream::default();
        let done = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "tool", "step_index": 2, "state": "ERROR",
                                "tool_name": "Run", "tool_info": {"error": {"message": "nope"}}}
            }),
        );
        let result = done.iter().find(|e| e.is(kind::TOOL_RESULT)).unwrap();
        assert!(!result.ok);
        assert_eq!(result.result, json!("nope"));
    }

    #[test]
    fn a_turn_always_ends() {
        let mut stream = Stream::default();
        let events = feed(
            &mut stream,
            json!({
                "event": "result",
                "result": {
                    "conversation_id": "c9", "status": "ERROR", "error": "429 quota"
                }
            }),
        );
        assert_eq!(events[0].fault, crate::events::LIMIT);
        assert!(events[1].is(kind::END));
        assert_eq!(events[0].native["conversation_id"], "c9");
        assert_eq!(events[1].native["conversation_id"], "c9");
    }

    #[test]
    fn an_aborted_turn_cannot_leak_step_state_into_the_next_one() {
        let mut stream = Stream::default();
        feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "agent_response", "step_index": 0,
                                "state": "ACTIVE", "text_delta": "stale "}
            }),
        );
        let first_call = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "tool", "step_index": 1, "state": "ACTIVE",
                                "tool_name": "OldTool", "tool_info": {}}
            }),
        );
        assert!(first_call.iter().any(|event| event.is(kind::TOOL_CALL)));
        feed(
            &mut stream,
            json!({"event": "result", "result": {"status": "ERROR", "error": "aborted"}}),
        );

        let text = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "agent_response", "step_index": 0,
                                "state": "DONE", "text_delta": "fresh"}
            }),
        );
        assert_eq!(
            text.iter().find(|event| event.is(kind::TEXT)).unwrap().text,
            "fresh"
        );
        let second_call = feed(
            &mut stream,
            json!({
                "event": "step_update",
                "step_update": {"step_type": "tool", "step_index": 1, "state": "ACTIVE",
                                "tool_name": "NewTool", "tool_info": {}}
            }),
        );
        assert!(
            second_call.iter().any(|event| event.is(kind::TOOL_CALL)),
            "a reused step index must still announce the new turn's tool"
        );
    }

    #[test]
    fn agy_is_told_things_under_event_not_type() {
        let line: Value = serde_json::from_str(&user_line("hi")).unwrap();
        assert_eq!(line["event"], "user");
        assert_eq!(line["message"]["content"], "hi");
    }
}
