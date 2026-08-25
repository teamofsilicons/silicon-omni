//! What a client and the daemon say to each other.
//!
//! One JSON object per line, both ways, over a Unix socket. That is the whole
//! protocol: it is trivial to speak from any language, it is readable when you
//! `nc` the socket to see what is happening, and it never needs a schema
//! compiler. The Python package, the CLI and the Rust client are all the same
//! few lines of socket code around what is defined here.
//!
//! A client sends a [`Request`] and gets exactly one [`Reply`] with the same
//! `id`. Anything without an `id` is a [`Frame`]: an event on a session this
//! connection has opened. The two are interleaved freely, so a client must read
//! lines in a loop rather than assuming the next line answers its question.
//!
//! Unknown fields are ignored and unknown ops are refused by name, so a newer
//! client against an older daemon fails with a sentence rather than a hang.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::Snapshot;
use crate::events::Event;

/// The current protocol. Bumped when a client could be misled by the answer,
/// never for a purely additive field.
pub const PROTOCOL: u32 = 1;

/// What a client asks for.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Echoed back on the reply. Any number the client likes.
    #[serde(default)]
    pub id: i64,
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<String>>,
    /// The opening client's process directory. Used only when a session has
    /// never pinned its own working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Replay events from this position on `open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    /// What to change, for `set`; which account question, for `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub what: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl Request {
    pub fn new(id: i64, op: &str) -> Self {
        Request {
            id,
            op: op.into(),
            session: None,
            text: None,
            providers: None,
            cwd: None,
            from: None,
            what: None,
            value: Value::Null,
            provider: None,
        }
    }

    pub fn on(mut self, session: &str) -> Self {
        self.session = Some(session.to_string());
        self
    }

    pub fn saying(mut self, text: &str) -> Self {
        self.text = Some(text.to_string());
        self
    }

    pub fn about(mut self, what: &str, value: Value) -> Self {
        self.what = Some(what.to_string());
        self.value = value;
        self
    }
}

/// The one answer to one [`Request`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub id: i64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok(id: i64, result: Value) -> Self {
        Reply {
            id,
            ok: true,
            result,
            error: None,
        }
    }

    pub fn failed(id: i64, error: impl Into<String>) -> Self {
        Reply {
            id,
            ok: false,
            result: Value::Null,
            error: Some(error.into()),
        }
    }
}

/// Something the daemon says on its own: an event, or a session ending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    pub stream: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
}

impl Frame {
    /// An event, and where the session stands as it goes out.
    ///
    /// The snapshot rides along because otherwise every client in every
    /// language has to re-derive `status` from the event stream — the same
    /// rules, written three more times, drifting three different ways. One
    /// short object per event is a much better trade than that.
    pub fn event(session: &str, event: Event, snapshot: Snapshot) -> Self {
        Frame {
            stream: "event".into(),
            session: session.into(),
            event: Some(event),
            snapshot: Some(snapshot),
        }
    }

    /// The session has stopped; nothing more will arrive for it.
    pub fn gone(session: &str, snapshot: Snapshot) -> Self {
        Frame {
            stream: "gone".into(),
            session: session.into(),
            event: None,
            snapshot: Some(snapshot),
        }
    }
}

/// A line from the daemon is one of these. Told apart by which keys it has,
/// so a reader never has to guess.
pub enum Incoming {
    Reply(Reply),
    Frame(Box<Frame>),
}

pub fn read_line(line: &str) -> Option<Incoming> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("stream").is_some() {
        return serde_json::from_value(value)
            .ok()
            .map(Box::new)
            .map(Incoming::Frame);
    }
    serde_json::from_value(value).ok().map(Incoming::Reply)
}

/// Every op the daemon understands, so a refusal can name the alternatives.
pub const OPS: &[&str] = &[
    "ping",
    "open",
    "send",
    "set",
    "stop",
    "detach",
    "status",
    "events",
    "sessions",
    "providers",
    "dial",
    "account",
    "test",
    "shutdown",
];

#[cfg(test)]
mod tests {
    use crate::choose::Ask;
    use super::*;
    use crate::events::event_type;

    fn snap() -> Snapshot {
        Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            ask: Ask::intelligence(5),
            providers: vec![],
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: "/".into(),
            seq: 0,
            in_turn: false,
            queued: 0,
        }
    }

    #[test]
    fn a_request_is_one_line_of_json() {
        let asked = Request::new(1, "send").on("demo").saying("hi");
        let line = serde_json::to_string(&asked).unwrap();
        assert!(!line.contains('\n'));
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.session.as_deref(), Some("demo"));
        assert_eq!(back.text.as_deref(), Some("hi"));
        assert!(
            !line.contains("provider"),
            "unset fields are left off: {line}"
        );
    }

    #[test]
    fn a_reply_and_a_frame_are_told_apart_by_shape() {
        let reply = serde_json::to_string(&Reply::ok(7, serde_json::json!({"a": 1}))).unwrap();
        let frame =
            serde_json::to_string(&Frame::event("demo", Event::new(event_type::TEXT), snap()))
                .unwrap();
        assert!(matches!(read_line(&reply), Some(Incoming::Reply(r)) if r.id == 7));
        assert!(matches!(read_line(&frame), Some(Incoming::Frame(f)) if f.stream == "event"));
    }

    #[test]
    fn a_line_that_is_not_ours_is_ignored_rather_than_fatal() {
        assert!(read_line("not json").is_none());
        assert!(read_line("[]").is_none());
    }

    #[test]
    fn a_newer_client_does_not_break_an_older_daemon() {
        let raw = r#"{"id":1,"op":"send","session":"s","something_new":true}"#;
        let back: Request = serde_json::from_str(raw).unwrap();
        assert_eq!(back.op, "send");
        assert_eq!(back.cwd, None, "cwd is additive on the wire");
    }

    #[test]
    fn an_open_can_name_the_calling_process_directory() {
        let mut asked = Request::new(1, "open").on("demo");
        asked.cwd = Some("/client/work".into());
        let line = serde_json::to_string(&asked).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(back.cwd.as_deref(), Some("/client/work"));
    }

    #[test]
    fn an_event_survives_the_round_trip_whole() {
        let mut event = Event::new(event_type::TOOL_CALL);
        event.tool = "Bash".into();
        event.args.insert("command".into(), serde_json::json!("ls"));
        event.seq = 12;
        let line = serde_json::to_string(&Frame::event("demo", event, snap())).unwrap();
        let Some(Incoming::Frame(frame)) = read_line(&line) else {
            panic!("not a frame")
        };
        let back = frame.event.unwrap();
        assert_eq!(back.tool, "Bash");
        assert_eq!(back.args["command"], "ls");
        assert_eq!(back.seq, 12);
    }
}
