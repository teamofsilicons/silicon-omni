//! Which native session each provider holds for one omni session.
//!
//! omni session A may sit on claude session B and codex thread C. `synced` is
//! how far up the omni log that native session has already seen, so coming back
//! to a provider only replays the part it missed.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::shared::{clock, paths};

pub struct Meta {
    pub session_id: String,
    pub path: PathBuf,
    data: Map<String, Value>,
}

impl Meta {
    pub fn open(session_id: &str) -> Self {
        let path = paths::meta_file(session_id);
        let data = Self::load(&path).unwrap_or_else(|| {
            json!({"session": session_id, "created": clock::now(), "providers": {}})
                .as_object()
                .cloned()
                .unwrap_or_default()
        });
        Meta {
            session_id: session_id.to_string(),
            path,
            data,
        }
    }

    /// A torn or hand-mangled file starts over rather than bricking the session.
    fn load(path: &std::path::Path) -> Option<Map<String, Value>> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut data: Map<String, Value> = serde_json::from_str(&text).ok()?;
        if !data.get("providers").is_some_and(Value::is_object) {
            data.insert("providers".into(), json!({}));
        }
        Some(data)
    }

    /// Written whole or not at all: this is rewritten on every turn.
    pub fn save(&mut self) {
        self.data.insert("updated".into(), json!(clock::now()));
        let Some(parent) = self.path.parent() else {
            return;
        };
        let _ = std::fs::create_dir_all(parent);
        let staging = parent.join(format!(".{}.meta.{}", self.session_id, std::process::id()));
        let body = serde_json::to_string_pretty(&self.data).unwrap_or_default();
        if std::fs::write(&staging, body).is_ok() {
            let _ = std::fs::rename(&staging, &self.path);
        }
    }

    fn providers(&mut self) -> &mut Map<String, Value> {
        self.data
            .entry("providers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("providers is an object")
    }

    /// The native session id this provider holds, and how far it has been told.
    pub fn native(&mut self, provider: &str) -> (String, i64) {
        let entry = self
            .providers()
            .entry(provider)
            .or_insert_with(|| json!({"id": "", "synced": -1}));
        (
            entry["id"].as_str().unwrap_or_default().to_string(),
            entry["synced"].as_i64().unwrap_or(-1),
        )
    }

    pub fn bind(&mut self, provider: &str, native_id: &str) {
        self.native(provider);
        self.providers()[provider]["id"] = json!(native_id);
        self.save();
    }

    pub fn mark_synced(&mut self, provider: &str, seq: i64) {
        self.native(provider);
        self.providers()[provider]["synced"] = json!(seq);
        self.save();
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key).filter(|value| !value.is_null())
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key)?.as_str().map(str::to_string)
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.data.insert(key.into(), value);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_home;

    #[test]
    fn a_provider_starts_unknown_and_unsynced() {
        let _home = scratch_home("meta-fresh");
        assert_eq!(Meta::open("s").native("claude"), (String::new(), -1));
    }

    #[test]
    fn what_was_bound_survives_a_reopen() {
        let _home = scratch_home("meta-bind");
        let mut meta = Meta::open("s");
        meta.bind("claude", "uuid-1");
        meta.mark_synced("claude", 7);
        meta.set("cwd", json!("/tmp"));
        let mut back = Meta::open("s");
        assert_eq!(back.native("claude"), ("uuid-1".into(), 7));
        assert_eq!(back.get_str("cwd").as_deref(), Some("/tmp"));
    }

    #[test]
    fn a_mangled_file_starts_over_rather_than_bricking_the_session() {
        let _home = scratch_home("meta-torn");
        std::fs::create_dir_all(paths::sessions()).unwrap();
        std::fs::write(paths::meta_file("s"), "{not json at all").unwrap();
        let mut meta = Meta::open("s");
        assert_eq!(meta.native("claude"), (String::new(), -1));
        meta.bind("claude", "uuid-2");
        assert_eq!(Meta::open("s").native("claude").0, "uuid-2");
    }
}
