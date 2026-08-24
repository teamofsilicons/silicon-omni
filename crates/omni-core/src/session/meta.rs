//! Which native session each provider holds for one omni session.
//!
//! omni session A may sit on claude session B and codex thread C. `synced` is
//! how far up the omni log that native session has already seen, so coming back
//! to a provider only replays the part it missed.

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::{fs, io};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::shared::{clock, paths};

const PROVIDERS: &str = "providers";
const SETTINGS: &str = "settings";
const PENDING: &str = "pending";

/// A user message accepted by omni but not yet represented by a durable
/// START/INJECTED event. The id, rather than the text, reconciles duplicate
/// messages after a crash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMessage {
    pub id: String,
    pub text: String,
}

pub struct Meta {
    pub session_id: String,
    pub path: PathBuf,
    data: Map<String, Value>,
    load_error: Option<String>,
}

impl Meta {
    pub fn open(session_id: &str) -> Self {
        let path = paths::meta_file(session_id);
        let loaded = Self::load(&path);
        let load_error = loaded.as_ref().err().map(ToString::to_string);
        let data = loaded.ok().flatten().unwrap_or_else(|| {
            json!({
                "session": session_id,
                "created": clock::now(),
                "providers": {},
                "settings": {},
                "pending": [],
            })
            .as_object()
            .cloned()
            .unwrap_or_default()
        });
        // `mode` on a create call cannot repair state from an older release.
        // Do that as soon as the file is encountered, including malformed
        // files that are deliberately retained for diagnosis.
        if path.exists() {
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Meta {
            session_id: session_id.to_string(),
            path,
            data,
            load_error,
        }
    }

    /// Missing metadata means a new session. Existing metadata that cannot be
    /// read or parsed is an error: silently replacing it could discard native
    /// session ids, settings, or an acknowledged pending message.
    fn load(path: &std::path::Path) -> io::Result<Option<Map<String, Value>>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut data: Map<String, Value> = serde_json::from_str(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        match data.get(PROVIDERS) {
            None => {
                data.insert(PROVIDERS.into(), json!({}));
            }
            Some(Value::Object(providers)) => {
                for (name, entry) in providers {
                    let Some(entry) = entry.as_object() else {
                        return Err(invalid(format!("providers.{name} must be an object")));
                    };
                    if entry.get("id").is_some_and(|value| !value.is_string()) {
                        return Err(invalid(format!("providers.{name}.id must be text")));
                    }
                    if entry
                        .get("synced")
                        .is_some_and(|value| value.as_i64().is_none())
                    {
                        return Err(invalid(format!(
                            "providers.{name}.synced must be an integer"
                        )));
                    }
                }
            }
            Some(_) => return Err(invalid("providers must be an object")),
        }
        match data.get(SETTINGS) {
            None => {
                data.insert(SETTINGS.into(), json!({}));
            }
            Some(Value::Object(_)) => {}
            Some(_) => return Err(invalid("settings must be an object")),
        }
        match data.get(PENDING) {
            None => {
                data.insert(PENDING.into(), json!([]));
            }
            Some(Value::Array(pending)) => {
                let mut ids = std::collections::BTreeSet::new();
                for (index, value) in pending.iter().enumerate() {
                    let message: PendingMessage =
                        serde_json::from_value(value.clone()).map_err(|error| {
                            invalid(format!("pending[{index}] is invalid: {error}"))
                        })?;
                    if message.id.is_empty() {
                        return Err(invalid(format!("pending[{index}].id cannot be empty")));
                    }
                    if !ids.insert(message.id) {
                        return Err(invalid(format!(
                            "pending[{index}].id duplicates an earlier message"
                        )));
                    }
                }
            }
            Some(_) => return Err(invalid("pending must be an array")),
        }
        Ok(Some(data))
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Written whole or not at all: this is rewritten on every turn. The
    /// in-memory copy changes only after the replacement reached disk.
    pub fn save(&mut self) -> io::Result<()> {
        self.ensure_writable()?;
        let mut next = self.data.clone();
        next.insert("updated".into(), json!(clock::now()));
        self.write(&next)?;
        self.data = next;
        Ok(())
    }

    fn write(&self, data: &Map<String, Value>) -> io::Result<()> {
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "metadata path has no parent",
            ));
        };
        paths::ensure(parent)?;
        let staging = parent.join(format!(".{}.meta.{}", self.session_id, std::process::id()));
        let body = serde_json::to_string_pretty(data)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&staging)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&staging, &self.path)?;
            // Renaming is not itself a durable directory update. The parent
            // sync closes the crash window in which an acknowledged rewrite
            // could disappear after power loss.
            fs::File::open(parent)?.sync_all()
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&staging);
            return Err(error);
        }
        Ok(())
    }

    fn providers(data: &mut Map<String, Value>) -> &mut Map<String, Value> {
        data.entry(PROVIDERS)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("providers is an object")
    }

    fn settings(data: &mut Map<String, Value>) -> &mut Map<String, Value> {
        data.entry(SETTINGS)
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("settings is an object")
    }

    fn commit(&mut self, change: impl FnOnce(&mut Map<String, Value>)) -> io::Result<()> {
        self.ensure_writable()?;
        let mut next = self.data.clone();
        change(&mut next);
        next.insert("updated".into(), json!(clock::now()));
        self.write(&next)?;
        self.data = next;
        Ok(())
    }

    fn ensure_writable(&self) -> io::Result<()> {
        match &self.load_error {
            Some(error) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("existing session metadata could not be loaded: {error}"),
            )),
            None => Ok(()),
        }
    }

    /// The native session id this provider holds, and how far it has been told.
    pub fn native(&self, provider: &str) -> (String, i64) {
        let entry = self
            .data
            .get(PROVIDERS)
            .and_then(Value::as_object)
            .and_then(|providers| providers.get(provider));
        match entry {
            Some(entry) => (
                entry["id"].as_str().unwrap_or_default().to_string(),
                entry["synced"].as_i64().unwrap_or(-1),
            ),
            None => (String::new(), -1),
        }
    }

    pub fn bind(&mut self, provider: &str, native_id: &str) -> io::Result<()> {
        self.commit(|data| {
            let entry = Self::providers(data)
                .entry(provider)
                .or_insert_with(|| json!({"id": "", "synced": -1}));
            entry["id"] = json!(native_id);
        })
    }

    pub fn mark_synced(&mut self, provider: &str, seq: i64) -> io::Result<()> {
        self.commit(|data| {
            let entry = Self::providers(data)
                .entry(provider)
                .or_insert_with(|| json!({"id": "", "synced": -1}));
            entry["synced"] = json!(seq);
        })
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key).filter(|value| !value.is_null())
    }

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.get(key)?.as_str().map(str::to_string)
    }

    /// One durable chat setting. Kept under its own namespace so the active
    /// provider list cannot be mistaken for the top-level native-session map.
    pub fn setting(&self, key: &str) -> Option<&Value> {
        self.data
            .get(SETTINGS)
            .and_then(Value::as_object)
            .and_then(|settings| settings.get(key))
            .filter(|value| !value.is_null())
    }

    pub fn setting_str(&self, key: &str) -> Option<String> {
        self.setting(key)?.as_str().map(str::to_string)
    }

    /// Fill only settings that have never been pinned. The entire group is
    /// committed as one replacement, and the in-memory copy changes only
    /// after that replacement reaches disk.
    pub fn initialize_settings(&mut self, defaults: Map<String, Value>) -> io::Result<()> {
        self.commit(move |data| {
            let settings = Self::settings(data);
            for (key, value) in defaults {
                settings.entry(key).or_insert(value);
            }
        })
    }

    pub fn set_setting(&mut self, key: &str, value: Value) -> io::Result<()> {
        self.commit(|data| {
            Self::settings(data).insert(key.into(), value);
        })
    }

    pub fn set(&mut self, key: &str, value: Value) -> io::Result<()> {
        self.commit(|data| {
            data.insert(key.into(), value);
        })
    }

    /// Pending messages in the exact order they were acknowledged.
    pub fn pending(&self) -> Vec<PendingMessage> {
        self.data
            .get(PENDING)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|value| {
                serde_json::from_value(value.clone())
                    .expect("pending entries were validated when metadata was loaded")
            })
            .collect()
    }

    /// Durably accept a message before the caller is told it succeeded.
    pub fn enqueue(&mut self, text: &str) -> io::Result<PendingMessage> {
        let message = PendingMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.to_string(),
        };
        let value = serde_json::to_value(&message)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        self.commit(|data| {
            data.entry(PENDING)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("pending is an array")
                .push(value);
        })?;
        Ok(message)
    }

    /// Remove one message only after its correlated opening event is durable.
    pub fn complete(&mut self, id: &str) -> io::Result<()> {
        self.commit(|data| {
            if let Some(pending) = data.get_mut(PENDING).and_then(Value::as_array_mut) {
                pending.retain(|value| value.get("id").and_then(Value::as_str) != Some(id));
            }
        })
    }

    /// Repair the append-succeeded/cleanup-failed crash window. Correlation ids
    /// make this exact even when adjacent messages contain identical text.
    pub fn reconcile(&mut self, delivered: &std::collections::BTreeSet<String>) -> io::Result<()> {
        if delivered.is_empty()
            || !self
                .pending()
                .iter()
                .any(|message| delivered.contains(&message.id))
        {
            return Ok(());
        }
        self.commit(|data| {
            if let Some(pending) = data.get_mut(PENDING).and_then(Value::as_array_mut) {
                pending.retain(|value| {
                    value
                        .get("id")
                        .and_then(Value::as_str)
                        .is_none_or(|id| !delivered.contains(id))
                });
            }
        })
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
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
        meta.bind("claude", "uuid-1").unwrap();
        meta.mark_synced("claude", 7).unwrap();
        meta.set("cwd", json!("/tmp")).unwrap();
        meta.set_setting("active_providers", json!(["google"]))
            .unwrap();
        let back = Meta::open("s");
        assert_eq!(back.native("claude"), ("uuid-1".into(), 7));
        assert_eq!(back.get_str("cwd").as_deref(), Some("/tmp"));
        assert_eq!(back.setting("active_providers"), Some(&json!(["google"])));
        let raw: Value = serde_json::from_str(&fs::read_to_string(back.path).unwrap()).unwrap();
        assert_eq!(raw["providers"]["claude"]["id"], "uuid-1");
        assert_eq!(raw["settings"]["active_providers"], json!(["google"]));
    }

    #[test]
    fn a_mangled_file_is_preserved_and_refuses_to_discard_state() {
        let _home = scratch_home("meta-torn");
        std::fs::create_dir_all(paths::sessions()).unwrap();
        let path = paths::meta_file("s");
        std::fs::write(&path, "{not json at all").unwrap();
        let mut meta = Meta::open("s");
        assert!(meta.load_error().is_some());
        assert_eq!(meta.native("claude"), (String::new(), -1));
        assert!(meta.bind("claude", "uuid-2").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{not json at all");
    }

    #[test]
    fn structurally_invalid_namespaces_and_pending_entries_are_not_normalized_away() {
        let _home = scratch_home("meta-invalid-shapes");
        std::fs::create_dir_all(paths::sessions()).unwrap();
        for (session, body) in [
            ("providers", r#"{"providers": []}"#),
            ("provider-entry", r#"{"providers": {"claude": "uuid"}}"#),
            ("settings", r#"{"settings": []}"#),
            ("pending", r#"{"pending": {}}"#),
            ("pending-entry", r#"{"pending": [{"id": "kept"}]}"#),
        ] {
            let path = paths::meta_file(session);
            std::fs::write(&path, body).unwrap();
            let mut meta = Meta::open(session);
            assert!(meta.load_error().is_some(), "accepted {session}");
            assert!(meta.set("would", json!("overwrite")).is_err());
            assert_eq!(std::fs::read_to_string(path).unwrap(), body);
        }
    }

    #[test]
    fn failed_writes_roll_back_every_in_memory_change() {
        let _home = scratch_home("meta-write-failure");
        let mut meta = Meta::open("s");
        meta.bind("claude", "original").unwrap();
        meta.mark_synced("claude", 3).unwrap();
        meta.set("cwd", json!("/kept")).unwrap();
        meta.set_setting("level", json!(4)).unwrap();

        let valid_path = meta.path.clone();
        let blocker = paths::home().join("not-a-directory");
        fs::write(&blocker, "a regular file").unwrap();
        meta.path = blocker.join("s.meta.json");

        assert!(meta.bind("claude", "lost").is_err());
        assert!(meta.mark_synced("claude", 99).is_err());
        assert!(meta.set("cwd", json!("/lost")).is_err());
        assert!(meta.set_setting("level", json!(9)).is_err());
        assert_eq!(meta.native("claude"), ("original".into(), 3));
        assert_eq!(meta.get_str("cwd").as_deref(), Some("/kept"));
        assert_eq!(meta.setting("level"), Some(&json!(4)));

        meta.path = valid_path;
        meta.mark_synced("claude", 4).unwrap();
        assert_eq!(Meta::open("s").native("claude"), ("original".into(), 4));
    }

    #[test]
    fn initializing_settings_is_atomic_and_never_overwrites_a_pin() {
        let _home = scratch_home("meta-settings-init");
        let mut meta = Meta::open("s");
        meta.set_setting("cwd", json!("/kept")).unwrap();
        meta.initialize_settings(
            json!({"cwd": "/new", "level": 7})
                .as_object()
                .cloned()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(meta.setting_str("cwd").as_deref(), Some("/kept"));
        assert_eq!(meta.setting("level"), Some(&json!(7)));
    }

    #[test]
    fn pending_messages_are_fifo_exact_and_survive_a_reopen() {
        let _home = scratch_home("meta-pending");
        let mut meta = Meta::open("s");
        let first = meta.enqueue("same").unwrap();
        let second = meta.enqueue("same").unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(
            Meta::open("s").pending(),
            vec![first.clone(), second.clone()]
        );

        meta.complete(&first.id).unwrap();
        assert_eq!(Meta::open("s").pending(), vec![second]);
    }

    #[test]
    fn opening_and_rewriting_metadata_repairs_private_permissions() {
        let _home = scratch_home("meta-permissions");
        let mut meta = Meta::open("s");
        meta.set("cwd", json!("/tmp")).unwrap();
        std::fs::set_permissions(&meta.path, fs::Permissions::from_mode(0o644)).unwrap();

        let mut reopened = Meta::open("s");
        assert_eq!(
            fs::metadata(&reopened.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        reopened.set("cwd", json!("/var/tmp")).unwrap();
        assert_eq!(
            fs::metadata(&reopened.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !reopened
                .path
                .parent()
                .unwrap()
                .join(format!(".s.meta.{}", std::process::id()))
                .exists()
        );
    }
}
