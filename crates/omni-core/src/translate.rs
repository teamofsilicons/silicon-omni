//! Turning omni history into something a provider can be handed.
//!
//! Seeding only ever happens on a switch, so the history a provider is given is
//! by definition history it did not live through — usually another provider's.
//! Rather than fake structured tool calls it never made, omni renders that
//! activity as text that reads as what happened:
//!
//! ```text
//! [GoogleSearch: "kite festivals"]
//! [GoogleSearch result: 12 results ...]
//! ```
//!
//! The omni log itself is untouched, so nothing is lost: switching back to
//! Gemini replays Gemini's own session, and the bracket form only ever exists
//! inside the seed given to somebody else.
//!
//! Nothing is trimmed on the way in — not the oldest turns, not a long tool
//! result. A provider that arrives late gets the whole conversation, because
//! the one thing worse than a big seed is a provider confidently missing the
//! middle of it.

use serde_json::Value;

use crate::events::{Event, event_type};

pub const SEED_HEADER: &str = "Earlier in this conversation (carried over from another model, \
shown as a transcript — do not re-run anything in it):";

/// One turn of history as a role and the text that goes with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub role: &'static str,
    pub text: String,
}

/// A value as text. `quote` keeps a string quoted, the way an argument reads.
pub fn readable(value: &Value, quote: bool) -> String {
    match value {
        Value::String(text) if !quote => text.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// One history event as the text a foreign provider should read.
pub fn render(event: &Event) -> String {
    match event.event_type.as_str() {
        event_type::START | event_type::INJECTED | event_type::TEXT => event.text.clone(),
        event_type::TOOL_CALL => {
            let body = match event.args.len() {
                1 => readable(event.args.values().next().unwrap_or(&Value::Null), true),
                _ => readable(&Value::Object(event.args.clone()), false),
            };
            format!("[{}: {}]", event.tool, body)
        }
        event_type::TOOL_RESULT => {
            let tool = if event.tool.is_empty() {
                "tool"
            } else {
                &event.tool
            };
            let status = if event.ok { "" } else { " failed" };
            format!(
                "[{tool} result{status}: {}]",
                readable(&event.result, false)
            )
        }
        _ => String::new(),
    }
}

fn role_of(event: &Event) -> &'static str {
    match event.event_type.as_str() {
        event_type::START | event_type::INJECTED => "user",
        _ => "assistant",
    }
}

/// History as merged turns, ready to seed.
pub fn transcript(events: &[Event]) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for event in events {
        let text = render(event);
        if text.is_empty() {
            continue;
        }
        let role = role_of(event);
        match out.last_mut() {
            Some(last) if last.role == role => {
                last.text.push('\n');
                last.text.push_str(&text);
            }
            _ => out.push(Turn { role, text }),
        }
    }
    out
}

/// History as one message, for providers that accept nothing else.
pub fn flatten(events: &[Event], header: &str) -> String {
    let turns = transcript(events);
    if turns.is_empty() {
        return String::new();
    }
    let body = turns
        .iter()
        .map(|turn| format!("{}: {}", turn.role.to_uppercase(), turn.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{header}\n\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(tool: &str, args: Value) -> Event {
        let mut event = Event::new(event_type::TOOL_CALL);
        event.tool = tool.into();
        event.args = args.as_object().cloned().unwrap_or_default();
        event
    }

    #[test]
    fn a_lone_argument_reads_like_one() {
        assert_eq!(
            render(&call("GoogleSearch", json!({"query": "kite festivals"}))),
            r#"[GoogleSearch: "kite festivals"]"#
        );
    }

    #[test]
    fn several_arguments_stay_structured() {
        let text = render(&call("Edit", json!({"path": "a.py", "line": 3})));
        assert!(
            text.starts_with("[Edit: {") && text.contains("\"path\""),
            "{text}"
        );
    }

    #[test]
    fn a_failed_result_says_so() {
        let mut event = Event::new(event_type::TOOL_RESULT);
        event.tool = "Bash".into();
        event.ok = false;
        event.result = json!("no such file");
        assert_eq!(render(&event), "[Bash result failed: no such file]");
    }

    #[test]
    fn thinking_carries_nothing_across() {
        assert_eq!(render(&Event::new(event_type::THINKING)), "");
    }

    #[test]
    fn one_side_speaking_twice_is_one_turn() {
        let events = vec![
            Event::new(event_type::START).saying("hi"),
            Event::new(event_type::TEXT).saying("hello"),
            call("Bash", json!({"command": "ls"})),
            Event::new(event_type::THINKING),
            Event::new(event_type::TEXT).saying("done"),
            Event::new(event_type::INJECTED).saying("wait"),
        ];
        let turns = transcript(&events);
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[1].text, "hello\n[Bash: \"ls\"]\ndone");
        assert_eq!(turns[2].text, "wait");
    }

    #[test]
    fn nothing_to_say_is_nothing_at_all() {
        assert_eq!(flatten(&[], SEED_HEADER), "");
        assert_eq!(
            flatten(&[Event::new(event_type::THINKING)], SEED_HEADER),
            ""
        );
    }

    #[test]
    fn flattening_keeps_who_said_what() {
        let events = vec![
            Event::new(event_type::START).saying("hi"),
            Event::new(event_type::TEXT).saying("hello"),
        ];
        assert_eq!(
            flatten(&events, "EARLIER:"),
            "EARLIER:\n\nUSER: hi\n\nASSISTANT: hello"
        );
    }

    #[test]
    fn nothing_is_trimmed_however_long_it_is() {
        let long = "x".repeat(50_000);
        let mut event = Event::new(event_type::TOOL_RESULT);
        event.tool = "Read".into();
        event.result = json!(long);
        assert!(render(&event).len() > 50_000);
    }
}
