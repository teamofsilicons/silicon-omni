//! Claude Code's own session files, which is how omni seeds it.
//!
//! Claude keeps one JSONL per session at `~/.claude/projects/<slug>/<uuid>.jsonl`
//! where the slug is the working directory with every non-alphanumeric
//! character turned into a dash. Records form a linked list through
//! `parentUuid`.
//!
//! Writing that file before `--resume` is how a conversation that happened
//! somewhere else becomes one Claude remembers. Appending to a file Claude
//! already owns works just as well, so coming back only ever costs the part it
//! missed.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::shared::{clock, jsonl, paths};
use crate::translate::Turn;

const VERSION: &str = "2.1.237";

pub fn home() -> PathBuf {
    paths::dirs_home().join(".claude").join("projects")
}

pub fn slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn session_path(cwd: &str, session_id: &str) -> PathBuf {
    home().join(slug(cwd)).join(format!("{session_id}.jsonl"))
}

/// The last record's uuid — what the next record has to hang off.
fn tail_uuid(path: &Path) -> Option<String> {
    jsonl::read(path)
        .iter()
        .filter_map(|record| record.get("uuid").and_then(Value::as_str))
        .next_back()
        .map(str::to_string)
}

fn record(kind: &str, cwd: &str, session_id: &str, parent: Option<&str>, message: Value) -> Value {
    json!({
        "parentUuid": parent,
        "isSidechain": false,
        "userType": "external",
        "cwd": cwd,
        "sessionId": session_id,
        "version": VERSION,
        "gitBranch": "",
        "type": kind,
        "uuid": uuid::Uuid::new_v4().to_string(),
        "timestamp": clock::now(),
        "message": message,
    })
}

/// Append a transcript to Claude's session file, creating it if needed.
pub fn seed(cwd: &str, session_id: &str, turns: &[Turn], model: &str) -> std::io::Result<PathBuf> {
    let path = session_path(cwd, session_id);
    let mut parent = tail_uuid(&path);
    let mut written = Vec::with_capacity(turns.len());
    for turn in turns {
        let mut item = if turn.role == "user" {
            record(
                "user",
                cwd,
                session_id,
                parent.as_deref(),
                json!({"role": "user", "content": turn.text}),
            )
        } else {
            let mut item = record(
                "assistant",
                cwd,
                session_id,
                parent.as_deref(),
                json!({
                    "id": format!("msg_{}", uuid::Uuid::new_v4().simple().to_string()[..16].to_string()),
                    "type": "message",
                    "role": "assistant",
                    // whatever the dial said; omni names no model itself
                    "model": model,
                    "content": [{"type": "text", "text": turn.text}],
                    "stop_reason": Value::Null,
                    "stop_sequence": Value::Null,
                    "usage": {"input_tokens": 1, "output_tokens": 1},
                }),
            );
            item["requestId"] = json!("req_omni_seed");
            item
        };
        parent = item["uuid"].as_str().map(str::to_string);
        written.push(std::mem::take(&mut item));
    }
    jsonl::extend(&path, &written)?;
    Ok(path)
}

/// One stdin line for `--input-format stream-json`.
pub fn user_line(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}})
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_becomes_the_name_claude_gives_it() {
        assert_eq!(slug("/Users/me/my project"), "-Users-me-my-project");
        assert_eq!(slug("/tmp/a_b.c"), "-tmp-a-b-c");
    }

    #[test]
    fn seeded_records_form_one_chain() {
        let dir = std::env::temp_dir().join(format!("omni-claude-seed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chain.jsonl");
        // seed() writes under the real ~/.claude, so exercise the chaining
        // directly against a file we own.
        let turns = [
            Turn {
                role: "user",
                text: "hi".into(),
            },
            Turn {
                role: "assistant",
                text: "hello".into(),
            },
        ];
        let mut parent: Option<String> = None;
        let mut written = Vec::new();
        for turn in &turns {
            let item = record(
                "x",
                "/tmp",
                "s",
                parent.as_deref(),
                json!({"text": turn.text}),
            );
            parent = item["uuid"].as_str().map(str::to_string);
            written.push(item);
        }
        jsonl::extend(&path, &written).unwrap();
        let back = jsonl::read(&path);
        assert!(back[0]["parentUuid"].is_null());
        assert_eq!(back[1]["parentUuid"], back[0]["uuid"]);
        assert_eq!(tail_uuid(&path).as_deref(), back[1]["uuid"].as_str());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_user_line_is_what_stream_json_expects() {
        let line: Value = serde_json::from_str(&user_line("hi")).unwrap();
        assert_eq!(line["type"], "user");
        assert_eq!(line["message"]["content"][0]["text"], "hi");
    }
}
