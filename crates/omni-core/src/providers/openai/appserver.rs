//! A JSON-RPC client for `codex app-server`.
//!
//! One JSON object per line in both directions — no framing headers. Responses
//! are matched to requests by id; everything else is a notification and goes
//! straight to the listener. The server occasionally asks *us* something;
//! anything omni does not handle is declined politely so the server never waits
//! on a dead end.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

use crate::shared::proc::{LineProcess, Spawn};

const CLIENT: &str = "silicon-omni";

pub type OnNotify = Arc<dyn Fn(&str, &Value) + Send + Sync>;

#[derive(Debug)]
pub struct AppServerError(pub String);

impl std::fmt::Display for AppServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

type Pending = Arc<Mutex<BTreeMap<i64, mpsc::Sender<Value>>>>;

pub struct AppServer {
    proc: Arc<LineProcess>,
    ids: AtomicI64,
    pending: Pending,
}

impl AppServer {
    /// Start the server and shake hands. `on_notify` hears everything after.
    pub fn start(
        argv: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<String>,
        on_notify: OnNotify,
        on_exit: impl Fn(i32) + Send + Sync + 'static,
    ) -> Result<Self, AppServerError> {
        let pending: Pending = Arc::default();
        let heard = pending.clone();
        // Declining happens on the reader thread, which must not block on the
        // writer; it queues the refusal here and a writer thread sends it.
        let (decline_tx, decline_rx) = mpsc::channel::<Value>();
        let mut spawn = Spawn::new(argv).on_line(move |line| {
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                return;
            };
            let method = message.get("method").and_then(Value::as_str);
            match (method, message.get("id")) {
                // A server-to-client request omni has no answer for.
                (Some(method), Some(id)) => {
                    let _ = decline_tx.send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": format!("{method} not handled by omni")},
                    }));
                }
                (Some(method), None) => {
                    on_notify(method, message.get("params").unwrap_or(&Value::Null))
                }
                (None, Some(id)) => {
                    if let Some(id) = id.as_i64() {
                        let slot = heard.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
                        if let Some(tx) = slot {
                            let _ = tx.send(message);
                        }
                    }
                }
                _ => {}
            }
        });
        for (key, value) in env {
            spawn = spawn.env(&key, value);
        }
        if let Some(cwd) = cwd {
            spawn = spawn.cwd(cwd);
        }
        // The server is gone: fail everything waiting on it rather than hang.
        let orphaned = pending.clone();
        let proc = Arc::new(
            spawn
                .on_exit(move |code| {
                    let waiting =
                        std::mem::take(&mut *orphaned.lock().unwrap_or_else(|p| p.into_inner()));
                    for (_, tx) in waiting {
                        let _ = tx.send(json!({
                            "error": {"message": format!("codex app-server exited with {code}")}
                        }));
                    }
                    on_exit(code);
                })
                .start()
                .map_err(|err| {
                    AppServerError(format!("could not start codex app-server: {err}"))
                })?,
        );

        let writer = proc.clone();
        std::thread::spawn(move || {
            for reply in decline_rx {
                if !writer.send_line(&reply.to_string()) {
                    return;
                }
            }
        });

        let server = AppServer {
            proc,
            ids: AtomicI64::new(1),
            pending,
        };
        server.call(
            "initialize",
            json!({"clientInfo": {"name": CLIENT, "version": env!("CARGO_PKG_VERSION")}, "capabilities": {}}),
            Duration::from_secs(30),
        )?;
        Ok(server)
    }

    pub fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AppServerError> {
        let id = self.ids.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, tx);
        let mut body = json!({"jsonrpc": "2.0", "id": id, "method": method});
        if !params.is_null() {
            body["params"] = params;
        }
        if !self.proc.send_line(&body.to_string()) {
            self.pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
            return Err(AppServerError(format!(
                "codex app-server is not running (wanted {method})"
            )));
        }
        let Ok(message) = rx.recv_timeout(timeout) else {
            self.pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
            return Err(AppServerError(format!(
                "{method} timed out after {timeout:?}"
            )));
        };
        if let Some(error) = message.get("error") {
            let said = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            return Err(AppServerError(format!("{method}: {said}")));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    /// For calls that are allowed to fail — interrupts, best-effort probes.
    pub fn try_call(&self, method: &str, params: Value, timeout: Duration) -> Option<Value> {
        self.call(method, params, timeout).ok()
    }

    pub fn stop(&self) {
        self.proc.stop(Duration::from_secs(8));
    }

    pub fn alive(&self) -> bool {
        self.proc.alive()
    }
}
