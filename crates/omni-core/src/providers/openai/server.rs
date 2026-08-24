//! One warm `codex app-server`, shared by every session that wants it.
//!
//! The app server is not a session — it is a peer that hosts threads, and every
//! notification that belongs to a turn carries the `threadId` it belongs to.
//! So omni keeps one running per set of launch flags and rents threads from it:
//! joining costs a single call, and coming back to Codex after a switch costs
//! a `thread/resume` rather than a process launch.
//!
//! What flags the server was started with is the only thing that cannot be
//! changed per thread, so that is what the pool is keyed on. `cwd`, model,
//! effort and instructions all travel with the thread.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde_json::{Value, json};

use crate::providers::base::Emit;

use super::appserver::{AppServer, AppServerError};
use super::jail;
use super::stream::Stream;

/// Where a thread's events go, the parser state that names them, and which
/// turn is open on it — a message can only be steered into a turn that exists.
struct Route {
    emit: Emit,
    stream: Stream,
    turn: String,
}

pub struct Shared {
    pub flags: Vec<String>,
    server: AppServer,
    routes: Arc<Mutex<BTreeMap<String, Route>>>,
    /// Notifications that belong to the account rather than to a thread —
    /// `account/rateLimits/updated`, `account/login/completed`. The server
    /// volunteers them, so the newest of each is kept and answering costs
    /// nothing.
    notices: Arc<RwLock<BTreeMap<String, Value>>>,
    silenced: Mutex<std::collections::BTreeSet<String>>,
}

static POOL: Mutex<Option<BTreeMap<String, Arc<Shared>>>> = Mutex::new(None);

impl Shared {
    /// The warm server for these flags, started if there is not one already.
    pub fn get(flags: &[String]) -> Result<Arc<Shared>, AppServerError> {
        let key = flags.join(" ");
        let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
        let pool = pool.get_or_insert_with(BTreeMap::new);
        if let Some(live) = pool.get(&key).filter(|shared| shared.server.alive()) {
            return Ok(live.clone());
        }
        let shared = Arc::new(Shared::start(flags)?);
        pool.insert(key, shared.clone());
        Ok(shared)
    }

    fn start(flags: &[String]) -> Result<Shared, AppServerError> {
        let home = jail::build()
            .map_err(|err| AppServerError(format!("could not build codex jail: {err}")))?;
        let routes: Arc<Mutex<BTreeMap<String, Route>>> = Arc::default();
        let notices: Arc<RwLock<BTreeMap<String, Value>>> = Arc::default();
        let heard = routes.clone();
        let noted = notices.clone();
        let mut argv = vec!["codex".to_string(), "app-server".into(), "--stdio".into()];
        argv.extend(flags.iter().cloned());
        let server = AppServer::start(
            argv,
            vec![("CODEX_HOME".into(), home.to_string_lossy().into_owned())],
            None,
            Arc::new(move |method, params| {
                let Some(thread_id) = params.get("threadId").and_then(Value::as_str) else {
                    // Not about a thread, so it is about the account.
                    noted
                        .write()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(method.to_string(), params.clone());
                    return;
                };
                let mut routes = heard.lock().unwrap_or_else(|p| p.into_inner());
                let Some(route) = routes.get_mut(thread_id) else {
                    return;
                };
                match method {
                    "turn/started" => {
                        route.turn = params["turn"]["id"].as_str().unwrap_or("").to_string()
                    }
                    "turn/completed" => route.turn.clear(),
                    _ => {}
                }
                let events = route.stream.feed(method, params);
                let emit = route.emit.clone();
                drop(routes); // handlers must not run under the routing lock
                for event in events {
                    emit(event);
                }
            }),
            |_code| {},
        )?;
        Ok(Shared {
            flags: flags.to_vec(),
            server,
            routes,
            notices,
            silenced: Mutex::default(),
        })
    }

    pub fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AppServerError> {
        self.server.call(method, params, timeout)
    }

    pub fn try_call(&self, method: &str, params: Value, timeout: Duration) -> Option<Value> {
        self.server.try_call(method, params, timeout)
    }

    pub fn alive(&self) -> bool {
        self.server.alive()
    }

    /// Start listening for one thread's turns.
    pub fn attach(&self, thread_id: &str, model: &str, emit: Emit) {
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                thread_id.to_string(),
                Route {
                    emit,
                    stream: Stream::new(model),
                    turn: String::new(),
                },
            );
    }

    pub fn detach(&self, thread_id: &str) {
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(thread_id);
    }

    pub fn retune(&self, thread_id: &str, model: &str) {
        if let Some(route) = self
            .routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(thread_id)
        {
            route.stream.model = model.to_string();
        }
    }

    /// The turn currently open on a thread, if there is one.
    pub fn turn_of(&self, thread_id: &str) -> String {
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(thread_id)
            .map(|route| route.turn.clone())
            .unwrap_or_default()
    }

    /// Skills live outside CODEX_HOME, so the empty-folder trick misses them.
    /// Switching them off writes into the jail, so it is worth doing once per
    /// directory rather than once per session.
    pub fn silence_skills(&self, cwd: &str) {
        if !self
            .silenced
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(cwd.to_string())
        {
            return;
        }
        let listing = self
            .try_call(
                "skills/list",
                json!({"cwds": [cwd]}),
                Duration::from_secs(30),
            )
            .unwrap_or(Value::Null);
        for group in listing["data"].as_array().unwrap_or(&Vec::new()) {
            for skill in group["skills"].as_array().unwrap_or(&Vec::new()) {
                if skill.get("enabled").and_then(Value::as_bool) == Some(true) {
                    self.try_call(
                        "skills/config/write",
                        json!({"name": skill.get("name"), "enabled": false}),
                        Duration::from_secs(15),
                    );
                }
            }
        }
    }

    pub fn attached(&self) -> usize {
        self.routes.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// The last thing the server volunteered under this method, if anything.
    pub fn last(&self, method: &str) -> Option<Value> {
        self.notices
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(method)
            .cloned()
    }

    pub fn forget(&self, method: &str) {
        self.notices
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(method);
    }
}

/// Any warm server, for a question that is about the account rather than a
/// session. Never starts one: this is for reading what is already there.
pub fn any_live() -> Option<Arc<Shared>> {
    POOL.lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()?
        .values()
        .find(|shared| shared.alive())
        .cloned()
}

/// Put every warm server down. The daemon does this on the way out.
pub fn shutdown() {
    let pool = std::mem::take(&mut *POOL.lock().unwrap_or_else(|p| p.into_inner()));
    for (_, shared) in pool.into_iter().flatten() {
        shared.server.stop();
    }
}

/// Stop servers nothing is attached to. Called when a session goes cold.
pub fn reap_idle() -> usize {
    let mut pool = POOL.lock().unwrap_or_else(|p| p.into_inner());
    let Some(pool) = pool.as_mut() else { return 0 };
    let idle: Vec<String> = pool
        .iter()
        .filter(|(_, shared)| shared.attached() == 0 || !shared.alive())
        .map(|(key, _)| key.clone())
        .collect();
    for key in &idle {
        if let Some(shared) = pool.remove(key) {
            shared.server.stop();
        }
    }
    idle.len()
}

/// The settings a thread is started or resumed with.
pub fn settings(config: &crate::providers::base::Config) -> Value {
    let mut body = json!({
        "cwd": config.cwd,
        "sandbox": "danger-full-access",
        "approvalPolicy": "never",
    });
    for (key, value) in [
        ("model", &config.model),
        ("baseInstructions", &config.system_prompt),
        ("developerInstructions", &config.append_system_prompt),
    ] {
        if !value.is_empty() {
            body[key] = json!(value);
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::base::Config;

    #[test]
    fn a_thread_carries_where_it_runs_and_what_it_runs_as() {
        let config = Config {
            model: "m".into(),
            ..Config::default().at("/tmp")
        };
        let body = settings(&config);
        assert_eq!(body["cwd"], config.cwd);
        assert_eq!(body["model"], "m");
        assert_eq!(body["approvalPolicy"], "never");
    }

    #[test]
    fn an_unset_instruction_is_left_out_rather_than_sent_empty() {
        let body = settings(&Config::default());
        assert!(body.get("baseInstructions").is_none());
        assert!(body.get("model").is_none());
    }
}
