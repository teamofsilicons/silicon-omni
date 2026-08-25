//! Answering one request.
//!
//! Every op is a small function over the registry. Nothing here knows about
//! sockets — [`Daemon::answer`] takes a request and gives back what to say — so
//! the whole surface of the daemon can be tested without one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use omni_core::chat::Change;
use omni_core::providers::test as double;
use omni_core::session::Store;
use omni_core::wire::{OPS, PROTOCOL, Reply, Request};
use omni_core::{intelligence, providers};

use crate::registry::Registry;
use crate::wiring::Connection;

/// Long enough for descriptive names and UUIDs, short enough to stay well
/// inside every supported filesystem's single-component limit after suffixes.
const MAX_SESSION_ID_BYTES: usize = 128;

pub struct Daemon {
    pub registry: Registry,
    pub started: String,
    pub stopping: AtomicBool,
}

impl Default for Daemon {
    fn default() -> Self {
        Daemon {
            registry: Registry::default(),
            started: omni_core::shared::clock::now(),
            stopping: AtomicBool::new(false),
        }
    }
}

impl Daemon {
    pub fn answer(&self, request: &Request, conn: &Arc<Connection>) -> Reply {
        match self.dispatch(request, conn) {
            Ok(result) => Reply::ok(request.id, result),
            Err(why) => Reply::failed(request.id, why),
        }
    }

    fn dispatch(&self, request: &Request, conn: &Arc<Connection>) -> Result<Value, String> {
        // A session id becomes both a registry key and two filenames. Validate
        // it once, before the operation switch, so no present or future route
        // can accidentally reach either layer with a path supplied by a client.
        if let Some(session_id) = request.session.as_deref() {
            validate_session_id(session_id)?;
        }
        if self.stopping.load(Ordering::SeqCst) && request.op != "shutdown" {
            return Err("the omni daemon is shutting down".into());
        }
        match request.op.as_str() {
            "ping" => Ok(json!({
                "protocol": PROTOCOL,
                "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(),
                "started": self.started,
                "home": omni_core::shared::paths::home().to_string_lossy(),
            })),
            "open" => self.open(request, conn),
            "send" => self.send(request),
            "set" => self.set(request),
            "stop" => Ok(json!({"stopped": self.registry.stop(&self.named(request)?)})),
            "detach" => self.detach(request, conn),
            "status" => self.status(request),
            "events" => self.events(request),
            "sessions" => Ok(self.sessions()),
            "providers" => Ok(json!(providers::available(request.providers.as_deref()))),
            "dial" => self.dial(request),
            "account" => self.account(request),
            "test" => self.test(request),
            "shutdown" => {
                self.stopping.store(true, Ordering::SeqCst);
                Ok(json!({"stopping": true}))
            }
            other => Err(format!(
                "unknown op {other:?}; omni speaks: {}",
                OPS.join(", ")
            )),
        }
    }

    fn named(&self, request: &Request) -> Result<String, String> {
        request
            .session
            .clone()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("{} needs a session", request.op))
    }

    /// Attach this connection to a session, starting it if nobody had it open.
    fn open(&self, request: &Request, conn: &Arc<Connection>) -> Result<Value, String> {
        let session_id = self.named(request)?;
        let asked = request.providers.clone().filter(|named| !named.is_empty());
        let mut changes = settings(&request.value)?;
        // An explicit provider list is itself a setting. It seeds a new Chat,
        // and it must also overwrite the provider set when this is an attach to
        // a session the daemon is already keeping warm. A later queued
        // `active_inference_providers` call still wins because order is kept.
        if let Some(named) = &asked {
            changes.insert(0, Change::Providers(named.clone()));
        }
        let initial = asked.clone();
        // Settings travel with the open so they are in hand before the first
        // provider comes up. Sending them afterwards would start whatever the
        // session last used and then immediately replace it.
        let (live, replayed) = self.registry.open(
            &session_id,
            || initial.unwrap_or_else(|| providers::available(None)),
            changes,
            request.cwd.as_deref(),
            conn.clone(),
            request.from.unwrap_or(0),
        )?;
        let mut listening = conn.listening.lock().unwrap_or_else(|p| p.into_inner());
        if !listening.contains(&session_id) {
            listening.push(session_id.clone());
        }
        Ok(json!({
            "session": session_id,
            "replayed": replayed,
            "listeners": live.listeners(),
            "snapshot": live.handle.snapshot(),
        }))
    }

    fn send(&self, request: &Request) -> Result<Value, String> {
        let session_id = self.named(request)?;
        let text = request.text.clone().ok_or("send needs text")?;
        self.registry.send(&session_id, &text)?;
        Ok(json!({"accepted": true}))
    }

    fn set(&self, request: &Request) -> Result<Value, String> {
        let session_id = self.named(request)?;
        let what = request.what.clone().ok_or("set needs a `what`")?;
        let change = change_from(&what, &request.value)?;
        self.registry.set(&session_id, change)?;
        Ok(json!({"accepted": true}))
    }

    fn detach(&self, request: &Request, conn: &Arc<Connection>) -> Result<Value, String> {
        let session_id = self.named(request)?;
        if let Some(live) = self.registry.get(&session_id) {
            live.detach(conn.id);
        }
        conn.listening
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|name| name != &session_id);
        Ok(json!({"detached": session_id}))
    }

    fn status(&self, request: &Request) -> Result<Value, String> {
        let session_id = self.named(request)?;
        let live = self
            .registry
            .get(&session_id)
            .ok_or_else(|| gone(&session_id))?;
        Ok(json!({"snapshot": live.handle.snapshot(), "listeners": live.listeners()}))
    }

    /// A one-shot read of the log. For a client that wants history without
    /// holding a session open.
    fn events(&self, request: &Request) -> Result<Value, String> {
        let session_id = self.named(request)?;
        let from = request.from.unwrap_or(0);
        let store = Store::open(&session_id);
        if let Some(error) = store.load_error() {
            return Err(format!(
                "session {session_id:?} has a corrupt event log: {error}"
            ));
        }
        Ok(json!({"events": store.events(from)}))
    }

    fn sessions(&self) -> Value {
        let live: Vec<Value> = self
            .registry
            .snapshots()
            .into_iter()
            .map(|(snapshot, listeners)| json!({"snapshot": snapshot, "listeners": listeners}))
            .collect();
        json!({"sessions": live})
    }

    /// What each intelligence value from 0-10 resolves to for these providers.
    fn dial(&self, request: &Request) -> Result<Value, String> {
        let named = match request.providers.clone() {
            Some(named) if !named.is_empty() => named,
            _ => providers::available(None),
        };
        let table = intelligence::table(&named);
        if table.is_empty() {
            return Err(format!(
                "no dial for {named:?}: could not reach {} and nothing is cached",
                intelligence::remote()
            ));
        }
        Ok(json!(
            table
                .into_iter()
                .map(|(intelligence, rung)| (intelligence.to_string(), rung))
                .collect::<std::collections::BTreeMap<_, _>>()
        ))
    }

    fn account(&self, request: &Request) -> Result<Value, String> {
        let name = request.provider.clone().ok_or("account needs a provider")?;
        let account = providers::account(&name).ok_or_else(|| format!("no provider {name:?}"))?;
        let what = request.what.as_deref().unwrap_or("auth_status");
        Ok(match what {
            "auth_status" => json!(providers::auth_status(&name)),
            "installed" => json!(account.installed()),
            "limits" => match providers::auth_status(&name).as_str() {
                "authenticated" => account.limits(),
                _ => json!("unauthenticated"),
            },
            "start_auth" => json!(account.start_auth()),
            "finish_auth" => {
                let code = request.text.clone().unwrap_or_default();
                json!(account.finish_auth(&code))
            }
            "forget" => {
                providers::forget(&name);
                json!(true)
            }
            other => return Err(format!("nothing called {other:?} to ask an account")),
        })
    }

    /// Driving the built-in double, so a test in any language can reach it.
    fn test(&self, request: &Request) -> Result<Value, String> {
        let what = request.what.as_deref().unwrap_or("install");
        match what {
            #[cfg(test)]
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(
                    request.value["milliseconds"].as_u64().unwrap_or(0),
                ));
                Ok(json!(true))
            }
            "install" => {
                let names = request.providers.clone().unwrap_or_default();
                let rungs: Vec<intelligence::Rung> = serde_json::from_value(
                    request.value.get("rungs").cloned().unwrap_or(json!([])),
                )
                .map_err(|err| format!("bad rungs: {err}"))?;
                Ok(json!(double::install(&names, &rungs)))
            }
            "forget_all" => {
                double::forget_all();
                Ok(json!(true))
            }
            "installed" => Ok(json!(double::installed())),
            _ => self.drive(what, request),
        }
    }

    fn drive(&self, what: &str, request: &Request) -> Result<Value, String> {
        let name = request.provider.clone().ok_or("that needs a provider")?;
        let live = double::running(&name)
            .ok_or_else(|| format!("no test provider called {name:?} is running"))?;
        let value = &request.value;
        Ok(match what {
            "knobs" => {
                let was = live.knobs();
                live.set_knobs(double::Knobs {
                    autoreply: value["autoreply"].as_bool().unwrap_or(was.autoreply),
                    tunable: value["tunable"].as_bool().unwrap_or(was.tunable),
                    defer: value["defer"].as_bool().unwrap_or(was.defer),
                });
                let now = live.knobs();
                json!({"autoreply": now.autoreply, "tunable": now.tunable, "defer": now.defer})
            }
            "reply" => {
                live.reply(request.text.as_deref().unwrap_or(""));
                json!(true)
            }
            "fail" => {
                live.fail(
                    value["kind"].as_str().unwrap_or(""),
                    value["error"].as_str().unwrap_or(""),
                    value["ends"].as_bool().unwrap_or(false),
                );
                json!(true)
            }
            "state" => json!({
                "sent": live.sent(),
                "heard": live.heard(),
                "resumed": live.resumed(),
                "retuned": live.retuned(),
                "up": live.up(),
            }),
            other => return Err(format!("nothing called {other:?} to do to a test provider")),
        })
    }
}

/// The settings an `open` carries, in the order they were asked for.
fn settings(value: &Value) -> Result<Vec<Change>, String> {
    let Some(asked) = value.as_array() else {
        return Ok(Vec::new());
    };
    asked
        .iter()
        .map(|item| {
            let what = item
                .get("what")
                .and_then(Value::as_str)
                .ok_or("a setting needs a `what`")?;
            change_from(what, item.get("value").unwrap_or(&Value::Null))
        })
        .collect()
}

fn change_from(what: &str, value: &Value) -> Result<Change, String> {
    Ok(match what {
        "providers" => Change::Providers(strings(value)?),
        "level" | "intelligence" => {
            Change::Intelligence(value.as_i64().ok_or("intelligence takes a number")?)
        }
        "system_prompt" => Change::SystemPrompt(text_of(value)?),
        "append_system_prompt" => Change::AppendSystemPrompt(text_of(value)?),
        "subagents" => Change::Subagents(flag(value)?),
        "mcp" => Change::Mcp(flag(value)?),
        "autoremove" => Change::Autoremove(flag(value)?),
        "cwd" => Change::Cwd(text_of(value)?),
        other => return Err(format!("nothing called {other:?} can be set")),
    })
}

fn gone(session_id: &str) -> String {
    format!("session {session_id:?} is not open; open it first")
}

fn strings(value: &Value) -> Result<Vec<String>, String> {
    serde_json::from_value(value.clone()).map_err(|_| "that takes a list of names".into())
}

fn text_of(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        other => Err(format!("that takes text, not {other}")),
    }
}

fn flag(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "that takes true or false".into())
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() {
        return Err("invalid session id: it cannot be empty".into());
    }
    if session_id.len() > MAX_SESSION_ID_BYTES {
        return Err(format!(
            "invalid session id: it cannot exceed {MAX_SESSION_ID_BYTES} bytes"
        ));
    }
    if std::path::Path::new(session_id).is_absolute()
        || session_id.contains('/')
        || session_id.contains('\\')
    {
        return Err("invalid session id: paths and separators are not allowed".into());
    }
    if session_id.starts_with('.') || session_id.ends_with('.') || session_id.contains("..") {
        return Err("invalid session id: dot traversal is not allowed".into());
    }
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "invalid session id: use only ASCII letters, numbers, dots, hyphens, and underscores"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    use omni_core::providers::test as double;
    use omni_core::testing::scratch_home;

    #[test]
    fn normal_session_ids_are_preserved() {
        for session_id in [
            "demo",
            "release-2026_08.24",
            "550e8400-e29b-41d4-a716-446655440000",
        ] {
            assert_eq!(validate_session_id(session_id), Ok(()), "{session_id:?}");
        }
    }

    #[test]
    fn unsafe_session_ids_are_rejected_at_the_request_boundary() {
        let daemon = Daemon::default();
        let (ours, _peer) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, ours);
        let overlong = "a".repeat(MAX_SESSION_ID_BYTES + 1);
        let unsafe_ids = [
            "",
            "/tmp/escape",
            "../escape",
            "..\\escape",
            "nested/session",
            ".",
            "..",
            ".hidden",
            "trailing.",
            "two..dots",
            "space name",
            "colon:name",
            "café",
            &overlong,
        ];

        for session_id in unsafe_ids {
            // `events` is the route that otherwise opens the session file
            // directly, even when nothing is live in the registry.
            let reply = daemon.answer(&Request::new(7, "events").on(session_id), &conn);
            assert!(!reply.ok, "accepted unsafe id {session_id:?}");
            assert!(
                reply
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("invalid session id:")),
                "unexpected refusal for {session_id:?}: {:?}",
                reply.error
            );
        }
        assert!(daemon.registry.snapshots().is_empty());
    }

    #[test]
    fn open_reports_corrupt_metadata_instead_of_acknowledging_and_overwriting_it() {
        let _home = scratch_home("serve-corrupt-meta");
        std::fs::create_dir_all(omni_core::shared::paths::sessions()).unwrap();
        let path = omni_core::shared::paths::meta_file("s");
        std::fs::write(&path, "{keep this malformed state").unwrap();
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let daemon = Daemon::default();
        let (ours, _peer) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, ours);
        let mut request = Request::new(1, "open").on("s");
        request.providers = Some(names);

        let reply = daemon.answer(&request, &conn);

        assert!(!reply.ok);
        assert!(reply.error.unwrap().contains("metadata"));
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "{keep this malformed state"
        );
        assert!(daemon.registry.get("s").is_none());
        daemon.registry.shutdown();
    }

    #[test]
    fn set_and_send_return_failures_when_the_local_journal_cannot_commit() {
        let _home = scratch_home("serve-durable-errors");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let daemon = Daemon::default();
        let (ours, _peer) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, ours);

        for (session, request) in [
            (
                "set-fails",
                Request::new(2, "set")
                    .on("set-fails")
                    .about("intelligence", json!(8)),
            ),
            (
                "send-fails",
                Request::new(3, "send")
                    .on("send-fails")
                    .saying("must be durable"),
            ),
        ] {
            let mut open = Request::new(1, "open").on(session);
            open.providers = Some(names.clone());
            assert!(daemon.answer(&open, &conn).ok);
            // Meta stages each atomic rewrite at this exact private path. A
            // directory there simulates a local journal write failure without
            // changing the production path or relying on uid permissions.
            let staging = omni_core::shared::paths::sessions()
                .join(format!(".{session}.meta.{}", std::process::id()));
            std::fs::create_dir(&staging).unwrap();

            let reply = daemon.answer(&request, &conn);
            assert!(!reply.ok, "{session} was incorrectly acknowledged");
            let error = reply.error.unwrap();
            assert!(
                error.contains("saving") || error.contains("persistence"),
                "unexpected durable refusal: {error}"
            );
        }
        daemon.registry.shutdown();
    }

    #[test]
    fn one_shot_events_refuse_a_corrupt_log_instead_of_returning_its_prefix() {
        let _home = scratch_home("serve-corrupt-events");
        std::fs::create_dir_all(omni_core::shared::paths::sessions()).unwrap();
        let path = omni_core::shared::paths::session_file("s");
        let valid = omni_core::events::Event::new(omni_core::events::event_type::TEXT)
            .saying("known prefix")
            .to_value();
        std::fs::write(&path, format!("{valid}\n{{not valid json}}\n")).unwrap();
        let daemon = Daemon::default();
        let (ours, _peer) = UnixStream::pair().unwrap();
        let conn = Connection::new(1, ours);

        let reply = daemon.answer(&Request::new(1, "events").on("s"), &conn);

        assert!(!reply.ok);
        assert!(reply.result.is_null(), "no valid prefix was returned");
        assert!(reply.error.unwrap().contains("corrupt event log"));
        daemon.registry.shutdown();
    }
}
