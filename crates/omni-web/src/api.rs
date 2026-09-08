//! The doors, and what is behind each one.
//!
//! Every route is the same shape as an op on the daemon's socket, because the
//! bridge is a translation and not a second product. What it adds is the three
//! things a browser needs and a Unix socket does not: an omniauth to prove who
//! is asking, CORS so a page may ask at all, and an event stream because a
//! website cannot hold a socket open by itself.
//!
//! What it deliberately does not add is a way in. `start_auth`, `finish_auth`
//! and `forget` are refused here however good the token is: signing a provider
//! in is a thing you do at your own terminal, not something a page you visited
//! gets to start.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::bridge::{Bridge, Next};
use crate::http::{Answer, Req, Res, Sse, Stopper};
use crate::state::{Doorman, Grant, Refusal};

/// How long a quiet stream waits before it writes a keepalive and looks around.
const TICK: Duration = Duration::from_secs(5);
/// Ticks of quiet between keepalives.
const KEEPALIVE_EVERY: u32 = 4;

/// Ops a website may reach through the low-level passthrough.
///
/// The list is short on purpose. `account` is not on it because two of its
/// actions start a login; `test` is not because it installs providers; and
/// `shutdown` is not because a page should not be able to take the daemon
/// down under a terminal that is using it.
const PASSTHROUGH: &[&str] = &[
    "ping", "open", "send", "set", "stop", "detach", "status", "events", "sessions", "providers",
    "dial",
];

/// Account questions that only read.
const READABLE_ACCOUNT: &[&str] = &["auth_status", "status", "installed", "limits"];

pub struct Web {
    pub doorman: Arc<Doorman>,
    pub bridge: Arc<Bridge>,
    pub stopper: std::sync::Mutex<Option<Stopper>>,
    pub started: String,
    pub served: AtomicU64,
}

impl Web {
    pub fn new(doorman: Arc<Doorman>, bridge: Arc<Bridge>) -> Self {
        Web {
            doorman,
            bridge,
            stopper: std::sync::Mutex::new(None),
            started: omni_core::shared::clock::now(),
            served: AtomicU64::new(0),
        }
    }

    /// What the bridge says about itself to anyone who knocks.
    pub fn hello(&self) -> Value {
        json!({
            "omni": "web",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": silicon_omni::PROTOCOL,
            "instance": self.doorman.instance(),
            "port": self.doorman.port(),
            "started": self.started,
        })
    }

    pub fn answer(me: &Arc<Web>, request: Req) -> Answer {
        me.served.fetch_add(1, Ordering::Relaxed);
        let origin = request.header("origin").trim_end_matches('/').to_string();

        // A page on the open web can point a hostname at 127.0.0.1 and then
        // talk to whatever is listening. It cannot forge the Host header it
        // sends, so checking that the caller asked for this bridge by an
        // address that means "this machine" is what closes that door.
        if !me.addressed_here(&request) {
            return Answer::Done(me.dress(
                &origin,
                Res::json(
                    403,
                    &json!({
                        "error": format!(
                            "this bridge answers to localhost:{port} and 127.0.0.1:{port} only",
                            port = me.doorman.port()
                        )
                    }),
                ),
            ));
        }

        if request.method == "OPTIONS" {
            return Answer::Done(me.dress(&origin, Res::new(204)));
        }

        let route = (request.method.as_str(), request.path.as_str());
        match route {
            ("GET", "/") | ("GET", "/omni") => {
                Answer::Done(me.dress(&origin, Res::json(200, &me.hello())))
            }
            ("POST", "/connect") => Answer::Done(me.dress(&origin, me.connect(&request, &origin))),
            (_, path) if path.starts_with("/control/") => {
                Answer::Done(me.dress(&origin, me.control(&request)))
            }
            _ => me.guarded(me, request, origin),
        }
    }

    /// Everything that needs an omniauth.
    fn guarded(&self, me: &Arc<Web>, request: Req, origin: String) -> Answer {
        let grant = match self.doorman.check(request.bearer(), &origin) {
            Ok(grant) => grant,
            Err(refusal) => {
                let status = match refusal {
                    Refusal::Missing | Refusal::Unknown => 401,
                    Refusal::WrongOrigin { .. } => 403,
                    Refusal::LockedOut { .. } => 429,
                };
                return Answer::Done(self.dress(
                    &origin,
                    Res::json(status, &json!({"error": refusal.to_string()})),
                ));
            }
        };

        if request.method == "GET" && request.path == "/events" {
            return self.events(me, &request, &grant, &origin);
        }

        let response = match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/whoami") => self.whoami(&grant),
            ("POST", "/disconnect") => self.disconnect(&grant),
            ("GET", "/providers") => self.providers(&request, &grant),
            ("GET", "/dial") => self.dial(&request, &grant),
            ("GET", "/sessions") => self.sessions(&grant),
            ("GET", "/status") => self.status(&request, &grant),
            ("GET", "/history") => self.history(&request, &grant),
            ("GET", "/account") => self.account(&request, &grant),
            ("POST", "/open") => self.open(&request, &grant),
            ("POST", "/send") => self.send(&request, &grant),
            ("POST", "/set") => self.set(&request, &grant),
            ("POST", "/stop") => self.stop_session(&request, &grant),
            ("POST", "/detach") => self.detach(&request, &grant),
            ("POST", "/request") => self.passthrough(&request, &grant),
            ("GET", path) | ("POST", path) => Res::json(
                404,
                &json!({"error": format!("nothing at {path:?}; this bridge serves /omni, /connect, /open, /send, /set, /stop, /detach, /events, /history, /status, /sessions, /providers, /dial, /account")}),
            ),
            (method, _) => Res::json(405, &json!({"error": format!("{method} is not how you ask")})),
        };
        Answer::Done(self.dress(&origin, response))
    }

    // ------------------------------------------------------------- pairing

    fn connect(&self, request: &Req, origin: &str) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let code = body["code"].as_str().unwrap_or_default().trim().to_string();
        if code.is_empty() {
            return Res::json(
                400,
                &json!({"error": "send the code from `omni web connect`, as {\"code\": \"XULA-1998\"}"}),
            );
        }
        let name = body["name"].as_str().unwrap_or_default();
        match self.doorman.redeem(&code, name, origin) {
            Ok((token, grant)) => Res::json(
                200,
                &json!({
                    "omniauth": token,
                    "grant": describe(&grant),
                    "bridge": self.hello(),
                }),
            ),
            Err(refusal) => {
                let status = match refusal {
                    Refusal::LockedOut { .. } => 429,
                    _ => 401,
                };
                Res::json(status, &json!({"error": refusal.to_string()}))
            }
        }
    }

    fn whoami(&self, grant: &Grant) -> Res {
        Res::json(
            200,
            &json!({
                "grant": describe(grant),
                "bridge": self.hello(),
                "attached": self
                    .bridge
                    .attached(&grant.id)
                    .into_iter()
                    .map(|(session, readers)| json!({"session": session, "readers": readers}))
                    .collect::<Vec<_>>(),
            }),
        )
    }

    fn disconnect(&self, grant: &Grant) -> Res {
        self.bridge.forget(&grant.id);
        let taken = self.doorman.revoke(&grant.id);
        Res::json(200, &json!({"disconnected": !taken.is_empty()}))
    }

    // ---------------------------------------------------------------- reads

    fn providers(&self, request: &Req, grant: &Grant) -> Res {
        let only = list(request.query("only"));
        self.attempt(grant, |client| {
            client.providers(only).map(|names| json!(names))
        })
    }

    fn dial(&self, request: &Req, grant: &Grant) -> Res {
        let only = list(request.query("providers"));
        self.attempt(grant, |client| client.dial(only).map(|table| json!(table)))
    }

    fn sessions(&self, grant: &Grant) -> Res {
        self.attempt(grant, |client| {
            client.sessions().map(|live| json!({"sessions": live}))
        })
    }

    fn status(&self, request: &Req, grant: &Grant) -> Res {
        let Some(session) = named(request.query("session")) else {
            return needs_session();
        };
        self.attempt(grant, |client| {
            client
                .status(&session)
                .map(|(snapshot, listeners)| json!({"snapshot": snapshot, "listeners": listeners}))
        })
    }

    fn history(&self, request: &Req, grant: &Grant) -> Res {
        let Some(session) = named(request.query("session")) else {
            return needs_session();
        };
        let since: i64 = request.query("since").parse().unwrap_or(0);
        self.attempt(grant, |client| {
            client
                .events(&session, since)
                .map(|events| json!({"events": events}))
        })
    }

    fn account(&self, request: &Req, grant: &Grant) -> Res {
        let provider = request.query("provider").to_string();
        if provider.is_empty() {
            return Res::json(400, &json!({"error": "account needs a provider"}));
        }
        let asked = match request.query("what") {
            "" | "status" => "auth_status",
            other => other,
        };
        if !READABLE_ACCOUNT.contains(&asked) {
            return Res::json(
                403,
                &json!({
                    "error": format!(
                        "the web bridge only reads an account ({}); signing {provider} in happens at your terminal, with `omni account {provider} start-auth`",
                        READABLE_ACCOUNT.join(", ")
                    )
                }),
            );
        }
        let asked = asked.to_string();
        self.attempt(grant, |client| client.account(&provider, &asked, None))
    }

    // --------------------------------------------------------------- writes

    fn open(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let Some(session) = named(body["session"].as_str().unwrap_or_default()) else {
            return needs_session();
        };
        let providers = body["providers"]
            .as_array()
            .map(|named| named.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        let settings = match settings(&body["settings"]) {
            Ok(settings) => settings,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let from = body["from"].as_i64().unwrap_or(-1);
        match self
            .bridge
            .open(&grant.id, &session, providers, settings, from, true)
        {
            Ok((_, opened)) => Res::json(200, &json!(opened)),
            Err(why) => daemon_said(&why),
        }
    }

    fn send(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let Some(session) = named(body["session"].as_str().unwrap_or_default()) else {
            return needs_session();
        };
        let Some(text) = body["text"].as_str() else {
            return Res::json(400, &json!({"error": "send needs text"}));
        };
        // Sending to a session nobody is holding open would be refused by the
        // daemon. A website that only ever posts messages should not have to
        // know that, so the attachment is made on its behalf.
        if let Err(why) = self
            .bridge
            .open(&grant.id, &session, None, Vec::new(), -1, false)
        {
            return daemon_said(&why);
        }
        let text = text.to_string();
        self.attempt(grant, |client| {
            client
                .send(&session, &text)
                .map(|accepted| json!({"accepted": accepted}))
        })
    }

    fn set(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let Some(session) = named(body["session"].as_str().unwrap_or_default()) else {
            return needs_session();
        };
        let what = body["what"].as_str().unwrap_or_default().to_string();
        if what.is_empty() {
            return Res::json(400, &json!({"error": "set needs a `what`"}));
        }
        let value = body["value"].clone();
        self.attempt(grant, |client| {
            client
                .set(&session, &what, value)
                .map(|accepted| json!({"accepted": accepted}))
        })
    }

    fn stop_session(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let Some(session) = named(body["session"].as_str().unwrap_or_default()) else {
            return needs_session();
        };
        let answer = self.attempt(grant, |client| {
            client
                .stop(&session)
                .map(|stopped| json!({"stopped": stopped}))
        });
        self.bridge.detach(&grant.id, &session);
        answer
    }

    fn detach(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let Some(session) = named(body["session"].as_str().unwrap_or_default()) else {
            return needs_session();
        };
        Res::json(
            200,
            &json!({"detached": self.bridge.detach(&grant.id, &session)}),
        )
    }

    fn passthrough(&self, request: &Req, grant: &Grant) -> Res {
        let body = match request.json() {
            Ok(body) => body,
            Err(why) => return Res::json(400, &json!({"error": why})),
        };
        let op = body["op"].as_str().unwrap_or_default();
        if !PASSTHROUGH.contains(&op) {
            return Res::json(
                403,
                &json!({
                    "error": format!(
                        "the web bridge passes through {} — {op:?} is not one of them",
                        PASSTHROUGH.join(", ")
                    )
                }),
            );
        }
        let mut fields = body.as_object().cloned().unwrap_or_default();
        fields.insert("id".into(), json!(0));
        let asked: silicon_omni::Request = match serde_json::from_value(Value::Object(fields)) {
            Ok(asked) => asked,
            Err(error) => return Res::json(400, &json!({"error": error.to_string()})),
        };
        self.attempt(grant, |client| client.call(asked))
    }

    // ---------------------------------------------------------------- stream

    fn events(&self, me: &Arc<Web>, request: &Req, grant: &Grant, origin: &str) -> Answer {
        let Some(session) = named(request.query("session")) else {
            return Answer::Done(self.dress(origin, needs_session()));
        };
        /* A browser resumes an interrupted stream by sending back the id of
           the last event it saw. That is exactly `since`, so it wins over the
           query parameter, which is what the page asked for the first time. */
        let resumed: Option<i64> = request.header("last-event-id").parse().ok();
        let since = resumed.unwrap_or_else(|| request.query("since").parse().unwrap_or(-1));

        let (attachment, opened) =
            match self
                .bridge
                .open(&grant.id, &session, None, Vec::new(), -1, false)
            {
                Ok(both) => both,
                Err(why) => return Answer::Done(self.dress(origin, daemon_said(&why))),
            };
        // Where the live tail begins, noted before any history goes out, so
        // nothing that arrives while history is being read can be missed.
        let start = attachment.cursor();
        let reading = self.bridge.read(attachment);
        let client = match self.bridge.client(&grant.id) {
            Ok(client) => client,
            Err(why) => return Answer::Done(self.dress(origin, daemon_said(&why))),
        };

        let mut headers = vec![
            ("content-type".into(), "text/event-stream; charset=utf-8".into()),
            ("cache-control".into(), "no-cache, no-transform".into()),
            // Nothing between here and the browser should buffer a stream
            // whose whole point is arriving a token at a time.
            ("x-accel-buffering".into(), "no".into()),
        ];
        headers.extend(self.cors(origin));

        let bridge = self.bridge.clone();
        let stopper = self.stopper.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let hello = self.hello();
        let me = me.clone();

        Answer::Stream(
            headers,
            Box::new(move |sse: &mut Sse| {
                let _ = me;
                let reading = reading;
                let _ = sse.event(
                    None,
                    "open",
                    &json!({"opened": opened, "bridge": hello, "since": since}).to_string(),
                );

                // History first, straight from the log, then the live tail.
                // Anything read here may also be sitting in the ring, so the
                // highest sequence sent is remembered and the duplicate is
                // dropped rather than shown twice.
                let mut high_water = i64::MIN;
                if since >= 0 {
                    match client.events(&session, since) {
                        Ok(events) => {
                            for event in events {
                                high_water = high_water.max(event.seq);
                                let framed = json!({
                                    "stream": "event",
                                    "session": session,
                                    "event": event,
                                    "replay": true,
                                });
                                if sse.event(Some(high_water), "frame", &framed.to_string()).is_err() {
                                    bridge.finished_reading();
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            let _ = sse.event(
                                None,
                                "error",
                                &json!({"error": error.to_string()}).to_string(),
                            );
                        }
                    }
                }

                let mut cursor = start;
                let mut quiet = 0u32;
                loop {
                    if stopper.as_ref().is_some_and(Stopper::stopping) {
                        let _ = sse.event(
                            None,
                            "end",
                            &json!({"reason": "the bridge is shutting down"}).to_string(),
                        );
                        break;
                    }
                    let (next, resume) = reading.attachment().next(cursor, TICK);
                    cursor = resume;
                    match next {
                        Next::Frames(frames) => {
                            quiet = 0;
                            for held in frames {
                                let seq = held
                                    .frame
                                    .event
                                    .as_ref()
                                    .map(|event| event.seq)
                                    .unwrap_or(i64::MIN);
                                if held.frame.event.is_some() && seq <= high_water {
                                    continue;
                                }
                                if held.frame.event.is_some() {
                                    high_water = seq;
                                }
                                let body = serde_json::to_string(&held.frame).unwrap_or_default();
                                let id = held.frame.event.as_ref().map(|event| event.seq);
                                if sse.event(id, "frame", &body).is_err() {
                                    bridge.finished_reading();
                                    return;
                                }
                            }
                        }
                        Next::Lagged { missed, .. } => {
                            quiet = 0;
                            let _ = sse.event(
                                None,
                                "lag",
                                &json!({
                                    "missed": missed,
                                    "since": high_water,
                                    "hint": "this reader fell behind; refetch /history from `since`",
                                })
                                .to_string(),
                            );
                        }
                        Next::Quiet => {
                            quiet += 1;
                            if quiet % KEEPALIVE_EVERY == 0 && sse.keepalive().is_err() {
                                break;
                            }
                        }
                        Next::Over => {
                            let _ = sse.event(None, "end", &json!({"reason": "session"}).to_string());
                            break;
                        }
                    }
                }
                bridge.finished_reading();
            }),
        )
    }

    // --------------------------------------------------------------- control

    /// The half of the surface that answers to whoever can read `web.json`.
    fn control(&self, request: &Req) -> Res {
        if !self.doorman.is_control(request.bearer()) {
            return Res::json(
                401,
                &json!({"error": "control needs the token in ~/.omni/web.json; run this from `omni web` on the same machine"}),
            );
        }
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/control/connect") => {
                let code = self.doorman.mint_code();
                Res::json(
                    200,
                    &json!({
                        "code": code,
                        "expires_in": crate::state::CODE_TTL,
                        "bridge": self.hello(),
                    }),
                )
            }
            ("GET", "/control/status") => Res::json(
                200,
                &json!({
                    "bridge": self.hello(),
                    "pid": std::process::id(),
                    "grants": self.doorman.grants().iter().map(describe).collect::<Vec<_>>(),
                    "pending_codes": self.doorman.pending_codes(),
                    "streaming": self.bridge.streaming(),
                    "served": self.served.load(Ordering::Relaxed),
                }),
            ),
            ("POST", "/control/revoke") => {
                let body = request.json().unwrap_or_else(|_| json!({}));
                let taken = if body["all"].as_bool() == Some(true) {
                    let taken = self.doorman.revoke_all();
                    self.bridge.forget_all();
                    taken
                } else {
                    let which = body["which"].as_str().unwrap_or_default();
                    let taken = self.doorman.revoke(which);
                    for grant in &taken {
                        self.bridge.forget(&grant.id);
                    }
                    taken
                };
                Res::json(
                    200,
                    &json!({"revoked": taken.iter().map(describe).collect::<Vec<_>>()}),
                )
            }
            ("POST", "/control/stop") => {
                if let Some(stopper) = self.stopper.lock().unwrap_or_else(|p| p.into_inner()).clone()
                {
                    // Answer first, stop second: the caller wants to be told
                    // it worked, and the socket is about to go away.
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(50));
                        stopper.stop();
                    });
                }
                Res::json(200, &json!({"stopping": true}))
            }
            (method, path) => Res::json(
                404,
                &json!({"error": format!("nothing at {method} {path}")}),
            ),
        }
    }

    // ----------------------------------------------------------------- glue

    /// Run one call on this grant's daemon connection.
    fn attempt(
        &self,
        grant: &Grant,
        act: impl FnOnce(&silicon_omni::raw::Client) -> silicon_omni::Result<Value>,
    ) -> Res {
        let client = match self.bridge.client(&grant.id) {
            Ok(client) => client,
            Err(why) => return daemon_said(&why),
        };
        match act(&client) {
            Ok(value) => Res::json(200, &value),
            Err(error) => daemon_said(&error.to_string()),
        }
    }

    fn addressed_here(&self, request: &Req) -> bool {
        let host = request.header("host");
        if host.is_empty() {
            // HTTP/1.1 requires one. Nothing that reaches this bridge on
            // purpose omits it.
            return false;
        }
        let (name, port) = match host.rsplit_once(':') {
            // `[::1]:1998` splits correctly here; a bare `[::1]` does not
            // have a colon after the bracket and falls through below.
            Some((name, port)) if !name.ends_with('[') => (name, Some(port)),
            _ => (host, None),
        };
        let named_here = matches!(
            name.trim_start_matches('[').trim_end_matches(']'),
            "localhost" | "127.0.0.1" | "::1"
        );
        let right_port = match port {
            Some(port) => port.parse::<u16>() == Ok(self.doorman.port()),
            None => false,
        };
        named_here && right_port
    }

    fn cors(&self, origin: &str) -> Vec<(String, String)> {
        let mut headers = vec![
            // Every answer varies by Origin, including the ones that do not
            // set the allow header, or a cache will serve one site's answer
            // to another.
            ("vary".into(), "origin".into()),
            ("x-omni-instance".into(), self.doorman.instance().to_string()),
            ("x-omni-port".into(), self.doorman.port().to_string()),
        ];
        if !origin.is_empty() {
            headers.push(("access-control-allow-origin".into(), origin.to_string()));
        }
        headers.push((
            "access-control-allow-methods".into(),
            "GET, POST, OPTIONS".into(),
        ));
        headers.push((
            "access-control-allow-headers".into(),
            "authorization, content-type, last-event-id".into(),
        ));
        headers.push((
            "access-control-expose-headers".into(),
            "x-omni-instance, x-omni-port".into(),
        ));
        headers.push(("access-control-max-age".into(), "600".into()));
        /* No `allow-credentials`. The origin is echoed back rather than fixed,
           and the two together are the classic way to hand every site on the
           internet a logged-in session. Authority here is a bearer token,
           which a cross-origin page cannot be tricked into attaching. */
        headers
    }

    fn dress(&self, origin: &str, mut response: Res) -> Res {
        response.headers.extend(self.cors(origin));
        response
    }
}

fn describe(grant: &Grant) -> Value {
    json!({
        "id": grant.id,
        "name": grant.name,
        "origin": grant.origin,
        "issued": grant.issued,
        "expires_in": grant.expires_in(omni_core::shared::clock::epoch()),
    })
}

fn named(session: &str) -> Option<String> {
    let session = session.trim();
    (!session.is_empty()).then(|| session.to_string())
}

fn needs_session() -> Res {
    Res::json(400, &json!({"error": "that needs a session"}))
}

fn list(raw: &str) -> Option<Vec<String>> {
    let named: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    (!named.is_empty()).then_some(named)
}

/// Settings, in the order they were asked for.
///
/// An array keeps that order and is what the JS client sends. An object is
/// accepted because one setting is the common case and `{"model": "code"}` is
/// what somebody writes by hand.
fn settings(value: &Value) -> Result<Vec<(String, Value)>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let what = item["what"]
                    .as_str()
                    .ok_or("each setting needs a `what`")?
                    .to_string();
                Ok((what, item["value"].clone()))
            })
            .collect(),
        Value::Object(fields) => Ok(fields
            .iter()
            .map(|(what, value)| (what.clone(), value.clone()))
            .collect()),
        _ => Err("settings is an array of {what, value}, or an object".into()),
    }
}

/// Turn a daemon sentence into the status code that matches it.
fn daemon_said(why: &str) -> Res {
    let status = if why.contains("is not live") || why.contains("no session") {
        404
    } else if why.contains("already open") {
        409
    } else if why.contains("omni daemon") || why.contains("omnid") {
        502
    } else {
        400
    };
    Res::json(status, &json!({"error": why}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn web(port: u16) -> Arc<Web> {
        let file = std::env::temp_dir().join(format!(
            "omni-web-api-{}-{}.json",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        Arc::new(Web::new(
            Arc::new(Doorman::open(file, port)),
            Arc::new(Bridge::default()),
        ))
    }

    fn asking(method: &str, path: &str, host: &str) -> Req {
        let mut headers = BTreeMap::new();
        headers.insert("host".into(), host.to_string());
        Req {
            method: method.into(),
            path: path.into(),
            query: BTreeMap::new(),
            headers,
            body: Vec::new(),
        }
    }

    fn body(answer: Answer) -> (u16, Value) {
        let Answer::Done(response) = answer else {
            panic!("expected a plain answer, not a stream")
        };
        let value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        (response.status, value)
    }

    #[test]
    fn a_rebound_hostname_is_turned_away_before_anything_else_happens() {
        let me = web(1998);
        let (status, answer) = body(Web::answer(&me, asking("GET", "/omni", "evil.example:1998")));
        assert_eq!(status, 403);
        assert!(answer["error"].as_str().unwrap().contains("localhost:1998"));

        for host in ["localhost:1998", "127.0.0.1:1998", "[::1]:1998"] {
            let (status, _) = body(Web::answer(&me, asking("GET", "/omni", host)));
            assert_eq!(status, 200, "{host} is this machine");
        }
    }

    #[test]
    fn the_right_name_on_the_wrong_port_is_a_different_bridge() {
        let me = web(1998);
        let (status, _) = body(Web::answer(&me, asking("GET", "/omni", "localhost:1997")));
        assert_eq!(status, 403);
        let (status, _) = body(Web::answer(&me, asking("GET", "/omni", "localhost")));
        assert_eq!(status, 403, "no port at all is not this bridge either");
    }

    #[test]
    fn hello_is_open_to_anyone_so_a_page_can_find_the_bridge() {
        let me = web(1998);
        let (status, answer) = body(Web::answer(&me, asking("GET", "/omni", "localhost:1998")));
        assert_eq!(status, 200);
        assert_eq!(answer["omni"], "web");
        assert_eq!(answer["port"], 1998);
        assert!(answer["instance"].as_str().is_some_and(|id| !id.is_empty()));
    }

    #[test]
    fn everything_else_needs_an_omniauth() {
        let me = web(1998);
        for (method, path) in [
            ("GET", "/sessions"),
            ("GET", "/providers"),
            ("POST", "/send"),
            ("GET", "/events"),
        ] {
            let (status, answer) = body(Web::answer(&me, asking(method, path, "localhost:1998")));
            assert_eq!(status, 401, "{method} {path}");
            assert!(answer["error"].as_str().unwrap().contains("omniauth"));
        }
    }

    #[test]
    fn a_code_from_the_terminal_is_what_turns_into_a_token() {
        let me = web(1998);
        let code = me.doorman.mint_code();
        let mut request = asking("POST", "/connect", "localhost:1998");
        request.headers.insert("origin".into(), "https://example.com".into());
        request.body = json!({"code": code, "name": "Example"}).to_string().into_bytes();

        let (status, answer) = body(Web::answer(&me, request));
        assert_eq!(status, 200);
        let token = answer["omniauth"].as_str().unwrap().to_string();
        assert!(token.starts_with("omniauth_"));
        assert_eq!(answer["grant"]["origin"], "https://example.com");

        let mut authorised = asking("GET", "/whoami", "localhost:1998");
        authorised.headers.insert("origin".into(), "https://example.com".into());
        authorised
            .headers
            .insert("authorization".into(), format!("Bearer {token}"));
        let (status, answer) = body(Web::answer(&me, authorised));
        assert_eq!(status, 200);
        assert_eq!(answer["grant"]["name"], "Example");
    }

    #[test]
    fn a_wrong_code_does_not_say_which_part_was_wrong() {
        let me = web(1998);
        me.doorman.mint_code();
        let mut request = asking("POST", "/connect", "localhost:1998");
        request.body = json!({"code": "ZZZZ-1998"}).to_string().into_bytes();
        let (status, _) = body(Web::answer(&me, request));
        assert_eq!(status, 401);
    }

    #[test]
    fn signing_a_provider_in_is_not_something_a_website_gets_to_start() {
        let me = web(1998);
        let code = me.doorman.mint_code();
        let (token, _) = me.doorman.redeem(&code, "Example", "").unwrap();

        for what in ["start_auth", "finish_auth", "forget"] {
            let mut request = asking("GET", "/account", "localhost:1998");
            request.headers.insert("authorization".into(), format!("Bearer {token}"));
            request.query.insert("provider".into(), "claude".into());
            request.query.insert("what".into(), what.into());
            let (status, answer) = body(Web::answer(&me, request));
            assert_eq!(status, 403, "{what}");
            assert!(answer["error"].as_str().unwrap().contains("your terminal"));
        }
    }

    #[test]
    fn the_passthrough_carries_the_ops_it_says_it_does_and_no_others() {
        let me = web(1998);
        let code = me.doorman.mint_code();
        let (token, _) = me.doorman.redeem(&code, "Example", "").unwrap();

        for op in ["shutdown", "account", "test"] {
            let mut request = asking("POST", "/request", "localhost:1998");
            request.headers.insert("authorization".into(), format!("Bearer {token}"));
            request.body = json!({"op": op}).to_string().into_bytes();
            let (status, answer) = body(Web::answer(&me, request));
            assert_eq!(status, 403, "{op}");
            assert!(answer["error"].as_str().unwrap().contains(op));
        }
    }

    #[test]
    fn control_answers_only_to_whoever_could_read_the_file() {
        let me = web(1998);
        let (status, _) = body(Web::answer(&me, asking("GET", "/control/status", "localhost:1998")));
        assert_eq!(status, 401);

        let mut request = asking("GET", "/control/status", "localhost:1998");
        request
            .headers
            .insert("authorization".into(), format!("Bearer {}", me.doorman.control_token()));
        let (status, answer) = body(Web::answer(&me, request));
        assert_eq!(status, 200);
        assert_eq!(answer["bridge"]["port"], 1998);

        // A perfectly good omniauth is not a control token.
        let code = me.doorman.mint_code();
        let (token, _) = me.doorman.redeem(&code, "Example", "").unwrap();
        let mut asked = asking("POST", "/control/stop", "localhost:1998");
        asked.headers.insert("authorization".into(), format!("Bearer {token}"));
        let (status, _) = body(Web::answer(&me, asked));
        assert_eq!(status, 401);
    }

    #[test]
    fn a_preflight_is_answered_with_what_a_browser_needs_and_no_credentials() {
        let me = web(1998);
        let mut request = asking("OPTIONS", "/send", "localhost:1998");
        request.headers.insert("origin".into(), "https://example.com".into());
        let Answer::Done(response) = Web::answer(&me, request) else {
            panic!("a preflight is not a stream")
        };
        assert_eq!(response.status, 204);
        let headers: BTreeMap<_, _> = response.headers.into_iter().collect();
        assert_eq!(headers["access-control-allow-origin"], "https://example.com");
        assert!(headers["access-control-allow-headers"].contains("authorization"));
        assert_eq!(headers["vary"], "origin");
        assert!(
            !headers.contains_key("access-control-allow-credentials"),
            "an echoed origin plus credentials is the whole vulnerability"
        );
    }

    #[test]
    fn settings_are_read_ordered_or_plain() {
        let ordered = settings(&json!([
            {"what": "providers", "value": ["claude"]},
            {"what": "model", "value": {"how": "key", "key": "code"}}
        ]))
        .unwrap();
        assert_eq!(ordered[0].0, "providers");
        assert_eq!(ordered[1].0, "model");

        let plain = settings(&json!({"model": "code"})).unwrap();
        assert_eq!(plain, vec![("model".to_string(), json!("code"))]);

        assert!(settings(&Value::Null).unwrap().is_empty());
        assert!(settings(&json!("model=code")).is_err());
    }

    #[test]
    fn a_daemon_sentence_picks_the_status_that_matches_it() {
        assert_eq!(daemon_said("session \"x\" is not live").status, 404);
        assert_eq!(daemon_said("the omni daemon went away").status, 502);
        assert_eq!(daemon_said("session \"x\" is already open on this client").status, 409);
        assert_eq!(daemon_said("intelligence must be between 0 and 10").status, 400);
    }
}
