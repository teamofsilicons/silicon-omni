//! Claude's stdin control channel.
//!
//! `--input-format stream-json` accepts control requests alongside user
//! messages. They are how omni changes model or effort without tearing the
//! process down and re-reading the whole conversation, and how it reads usage
//! without spending a token.

use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::shared::proc::Spawn;

pub const BARE: &[&str] = &[
    super::CLI,
    "-p",
    "--output-format",
    "stream-json",
    "--input-format",
    "stream-json",
    "--verbose",
];

pub fn line(subtype: &str, request_id: &str, fields: Value) -> String {
    let mut request = json!({"subtype": subtype});
    if let (Some(request), Some(fields)) = (request.as_object_mut(), fields.as_object()) {
        for (key, value) in fields {
            request.insert(key.clone(), value.clone());
        }
    }
    json!({"type": "control_request", "request_id": request_id, "request": request}).to_string()
}

/// The `response` of a `control_response` line, if that is what this is.
pub fn answer(text: &str) -> Option<(String, Value)> {
    if !text.contains("\"control_response\"") {
        return None;
    }
    let data: Value = serde_json::from_str(text).ok()?;
    if data.get("type").and_then(Value::as_str) != Some("control_response") {
        return None;
    }
    let response = data.get("response")?.clone();
    let request_id = response.get("request_id")?.as_str()?.to_string();
    Some((request_id, response))
}

pub fn succeeded(response: &Value) -> bool {
    response.get("subtype").and_then(Value::as_str) == Some("success")
}

/// Ask a throwaway `claude` process one question. Costs no tokens.
pub fn ask(subtype: &str, fields: Value, timeout: Duration) -> Option<Value> {
    let (tx, rx) = mpsc::channel();
    let proc = Spawn::new(BARE.iter().copied())
        .on_line(move |text| {
            if let Some((_, response)) = answer(text) {
                let _ = tx.send(response);
            }
        })
        .start()
        .ok()?;
    proc.send_line(&line(subtype, "omni", fields));
    let response = rx.recv_timeout(timeout).ok();
    proc.stop(Duration::from_secs(5));
    let response = response?;
    succeeded(&response).then(|| response.get("response").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_control_request_carries_its_fields() {
        let sent: Value =
            serde_json::from_str(&line("set_model", "r1", json!({"model": "m"}))).unwrap();
        assert_eq!(sent["type"], "control_request");
        assert_eq!(sent["request_id"], "r1");
        assert_eq!(sent["request"]["subtype"], "set_model");
        assert_eq!(sent["request"]["model"], "m");
    }

    #[test]
    fn only_a_control_response_is_read_as_one() {
        assert!(answer(r#"{"type":"assistant"}"#).is_none());
        assert!(answer("not json").is_none());
        let (id, response) = answer(
            r#"{"type":"control_response","response":{"request_id":"r1","subtype":"success"}}"#,
        )
        .unwrap();
        assert_eq!(id, "r1");
        assert!(succeeded(&response));
    }

    #[test]
    fn a_refusal_is_not_a_success() {
        let (_, response) = answer(
            r#"{"type":"control_response","response":{"request_id":"r1","subtype":"error"}}"#,
        )
        .unwrap();
        assert!(!succeeded(&response));
    }
}
