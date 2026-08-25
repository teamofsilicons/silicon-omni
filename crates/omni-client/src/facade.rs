//! The event-first API shared in vocabulary with the Python package and CLI.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Client, DaemonInfo, Error, Event, IntelligenceRung, LiveSession, OpenOptions, Result, Session,
    Setting, Snapshot,
};

/// Daemon-owned Chat state using the canonical public field names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatState {
    pub session: String,
    pub status: String,
    #[serde(alias = "level")]
    pub intelligence: i64,
    pub providers: Vec<String>,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub cwd: String,
    pub seq: i64,
    pub in_turn: bool,
    pub queued: usize,
}

impl ChatState {
    /// Waiting with no turn open.
    pub fn idle(&self) -> bool {
        self.status == "waiting" && !self.in_turn
    }
}

impl From<Snapshot> for ChatState {
    fn from(snapshot: Snapshot) -> Self {
        ChatState {
            session: snapshot.session,
            status: snapshot.status,
            intelligence: snapshot.intelligence,
            providers: snapshot.providers,
            provider: snapshot.provider,
            model: snapshot.model,
            effort: snapshot.effort,
            cwd: snapshot.cwd,
            seq: snapshot.seq,
            in_turn: snapshot.in_turn,
            queued: snapshot.queued,
        }
    }
}

/// A live chat as reported by [`Inference::sessions`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveChat {
    pub snapshot: ChatState,
    pub listeners: usize,
}

impl From<LiveSession> for LiveChat {
    fn from(live: LiveSession) -> Self {
        LiveChat {
            snapshot: live.snapshot.into(),
            listeners: live.listeners,
        }
    }
}

/// The front door: discovery, provider accounts, daemon information and chats.
#[derive(Clone)]
pub struct Inference {
    client: Client,
    /// Anthropic through the `claude` CLI.
    pub claude: ProviderHandle,
    /// OpenAI through `codex app-server`.
    pub openai: ProviderHandle,
    /// Google through Antigravity's `agy` CLI.
    pub google: ProviderHandle,
}

impl fmt::Debug for Inference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Inference")
            .field("connected", &self.client.is_connected())
            .finish_non_exhaustive()
    }
}

impl Inference {
    /// Connect to the current daemon, starting it when necessary.
    pub fn connect() -> Result<Self> {
        Ok(Self::from_client(Client::connect()?))
    }

    /// Connect only when the current daemon is already listening.
    pub fn connect_existing() -> Result<Self> {
        Ok(Self::from_client(Client::connect_existing()?))
    }

    /// Connect to a specific daemon socket.
    pub fn connect_to(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::from_client(Client::connect_to(path)?))
    }

    /// Wrap an existing raw client. Useful for custom discovery and tests.
    pub fn from_client(client: Client) -> Self {
        Inference {
            claude: ProviderHandle::new(client.clone(), "claude"),
            openai: ProviderHandle::new(client.clone(), "openai"),
            google: ProviderHandle::new(client.clone(), "google"),
            client,
        }
    }

    /// Providers whose CLI is installed and authenticated.
    pub fn get_available_providers(&self, limit_to: Option<Vec<String>>) -> Result<Vec<String>> {
        self.client.providers(limit_to)
    }

    /// The cross-provider 0-10 intelligence dial.
    pub fn dial(&self, providers: Option<Vec<String>>) -> Result<BTreeMap<i64, IntelligenceRung>> {
        self.client.dial(providers)
    }

    /// Return a lazy chat handle, creating its durable session on first start.
    pub fn load_or_create_session(
        &self,
        session_id: impl Into<String>,
        providers: Option<Vec<String>>,
    ) -> Chat {
        Chat::new(self.client.clone(), session_id.into(), providers)
    }

    /// Every chat the daemon currently holds open.
    pub fn sessions(&self) -> Result<Vec<LiveChat>> {
        Ok(self
            .client
            .sessions()?
            .into_iter()
            .map(LiveChat::from)
            .collect())
    }

    /// Information about the persistent daemon.
    pub fn daemon(&self) -> Result<DaemonInfo> {
        self.client.ping()
    }

    /// Access to the advanced transport client.
    pub fn raw(&self) -> &Client {
        &self.client
    }
}

/// One provider's installation, authentication and quota controls.
#[derive(Clone)]
pub struct ProviderHandle {
    client: Client,
    name: &'static str,
}

impl fmt::Debug for ProviderHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProviderHandle")
            .field(&self.name)
            .finish()
    }
}

impl ProviderHandle {
    fn new(client: Client, name: &'static str) -> Self {
        ProviderHandle { client, name }
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn installed(&self) -> Result<bool> {
        value_as(
            "installed",
            self.client.account(self.name, "installed", None)?,
        )
    }

    pub fn auth_status(&self) -> Result<String> {
        value_as(
            "auth_status",
            self.client.account(self.name, "auth_status", None)?,
        )
    }

    pub fn available(&self) -> Result<bool> {
        Ok(self.installed()? && self.auth_status()? == "authenticated")
    }

    pub fn limits(&self) -> Result<Value> {
        self.client.account(self.name, "limits", None)
    }

    pub fn start_auth(&self) -> Result<String> {
        value_as(
            "start_auth",
            self.client.account(self.name, "start_auth", None)?,
        )
    }

    pub fn finish_auth(&self, code: &str) -> Result<String> {
        value_as(
            "finish_auth",
            self.client.account(self.name, "finish_auth", Some(code))?,
        )
    }

    pub fn forget(&self) -> Result<()> {
        self.client.account(self.name, "forget", None)?;
        Ok(())
    }
}

type Handler = Box<dyn FnMut(&Event) + Send + 'static>;

/// A persistent conversation handle.
///
/// Settings are lazy until [`Chat::start`] or [`Chat::send`]. Dropping or
/// detaching this handle leaves the daemon-side provider warm; [`Chat::stop`]
/// is the explicit global shutdown operation.
pub struct Chat {
    client: Client,
    session_id: String,
    providers: Option<Vec<String>>,
    cwd: Option<String>,
    pending: Vec<Setting>,
    attached: Option<Session>,
    state: Option<ChatState>,
    listeners: usize,
    seen: i64,
    started: bool,
    finished: bool,
    event_handlers: Vec<Handler>,
    log_handlers: Vec<Handler>,
}

impl fmt::Debug for Chat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Chat")
            .field("session_id", &self.session_id)
            .field("attached", &self.attached.is_some())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Chat {
    fn new(client: Client, session_id: String, providers: Option<Vec<String>>) -> Self {
        Chat {
            client,
            session_id,
            providers,
            cwd: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            pending: Vec::new(),
            attached: None,
            state: None,
            listeners: 0,
            seen: -1,
            started: false,
            finished: false,
            event_handlers: Vec::new(),
            log_handlers: Vec::new(),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Limit the providers this chat may use, at the next turn boundary.
    pub fn active_inference_providers(
        &mut self,
        providers: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<&mut Self> {
        let providers: Vec<String> = providers.into_iter().map(Into::into).collect();
        self.providers = Some(providers.clone());
        self.change("providers", json!(providers))
    }

    /// Set the shared 0-10 intelligence value.
    pub fn intelligence(&mut self, intelligence: i64) -> Result<&mut Self> {
        self.change("intelligence", json!(intelligence))
    }

    pub fn system_prompt(&mut self, text: impl Into<String>) -> Result<&mut Self> {
        self.change("system_prompt", json!(text.into()))
    }

    pub fn system_prompt_file(&mut self, path: impl AsRef<Path>) -> Result<&mut Self> {
        let text = read_text(path.as_ref())?;
        self.system_prompt(text)
    }

    pub fn append_system_prompt(&mut self, text: impl Into<String>) -> Result<&mut Self> {
        self.change("append_system_prompt", json!(text.into()))
    }

    pub fn append_system_prompt_file(&mut self, path: impl AsRef<Path>) -> Result<&mut Self> {
        let text = read_text(path.as_ref())?;
        self.append_system_prompt(text)
    }

    pub fn enable_subagents(&mut self) -> Result<&mut Self> {
        self.change("subagents", json!(true))
    }

    pub fn disable_subagents(&mut self) -> Result<&mut Self> {
        self.change("subagents", json!(false))
    }

    pub fn enable_mcp(&mut self) -> Result<&mut Self> {
        self.change("mcp", json!(true))
    }

    pub fn disable_mcp(&mut self) -> Result<&mut Self> {
        self.change("mcp", json!(false))
    }

    pub fn enable_autoremoving_unauthenticated_providers(&mut self) -> Result<&mut Self> {
        self.change("autoremove", json!(true))
    }

    pub fn disable_autoremoving_unauthenticated_providers(&mut self) -> Result<&mut Self> {
        self.change("autoremove", json!(false))
    }

    /// Pin the working directory for provider tools.
    pub fn cwd(&mut self, path: impl AsRef<Path>) -> Result<&mut Self> {
        let path = absolute(path.as_ref())?;
        let text = path.to_string_lossy().into_owned();
        self.cwd = Some(text.clone());
        self.change("cwd", json!(text))
    }

    /// Register a semantic event handler. It runs as events are consumed with
    /// [`Chat::next_event`] or [`Chat::events`].
    pub fn on_event(&mut self, handler: impl FnMut(&Event) + Send + 'static) -> &mut Self {
        self.event_handlers.push(Box::new(handler));
        self
    }

    /// Register a log handler. Daemon Events are the log; this receives the
    /// same ordered stream as `on_event`.
    pub fn logs(&mut self, handler: impl FnMut(&Event) + Send + 'static) -> &mut Self {
        self.log_handlers.push(Box::new(handler));
        self
    }

    /// Attach and replay the complete history.
    pub fn start(&mut self) -> Result<&mut Self> {
        self.start_since(0)
    }

    /// Attach and replay events beginning at `since`; `-1` means live only.
    pub fn start_since(&mut self, since: i64) -> Result<&mut Self> {
        if self.finished {
            return Err(Error::Stopped(format!(
                "session {:?} was stopped; load it again to continue",
                self.session_id
            )));
        }
        if self.attached.is_some() {
            return Ok(self);
        }
        let mut options = OpenOptions::new(self.session_id.clone()).from_seq(since);
        options.providers = self.providers.clone();
        options.cwd = self.cwd.clone();
        options.settings = self.pending.clone();
        let session = self.client.open(options)?;
        self.listeners = session.opened().listeners;
        self.state = Some(session.opened().snapshot.clone().into());
        if since < 0 {
            self.seen = self.seen.max(session.opened().snapshot.seq);
        } else {
            self.seen = self.seen.max(since - 1);
        }
        self.pending.clear();
        self.started = true;
        self.attached = Some(session);
        Ok(self)
    }

    /// Send a message, auto-attaching from the first unseen Event when needed.
    pub fn send(&mut self, text: &str) -> Result<()> {
        if self.finished {
            return Err(Error::Stopped(format!(
                "session {:?} was stopped; load it again to continue",
                self.session_id
            )));
        }
        if self.attached.is_none() {
            self.start_since(self.seen + 1)?;
        }
        let accepted = self
            .attached
            .as_ref()
            .expect("start populated the attached session")
            .send(text)?;
        if !accepted {
            return Err(Error::Daemon(format!(
                "session {:?} did not accept the message",
                self.session_id
            )));
        }
        Ok(())
    }

    /// Receive the next semantic Event. A stopped stream returns `None`.
    pub fn next_event(&mut self) -> Result<Option<Event>> {
        loop {
            if self.attached.is_none() {
                self.start_since(self.seen + 1)?;
            }
            let frame = self
                .attached
                .as_ref()
                .expect("start populated the attached session")
                .recv()?;
            if let Some(snapshot) = frame.snapshot {
                self.state = Some(snapshot.into());
            }
            let Some(event) = frame.event else {
                if frame.stream == "gone" {
                    self.attached.take();
                    self.finished = self
                        .state
                        .as_ref()
                        .is_some_and(|state| state.status == "stopped");
                    return Ok(None);
                }
                continue;
            };
            let event = canonical_event(event);
            self.seen = self.seen.max(event.seq);
            for handler in &mut self.event_handlers {
                handler(&event);
            }
            for handler in &mut self.log_handlers {
                handler(&event);
            }
            return Ok(Some(event));
        }
    }

    /// Receive one Event, returning `None` when the timeout expires or the
    /// session stops.
    pub fn next_event_timeout(&mut self, timeout: Duration) -> Result<Option<Event>> {
        if self.attached.is_none() {
            self.start_since(self.seen + 1)?;
        }
        let Some(frame) = self
            .attached
            .as_ref()
            .expect("start populated the attached session")
            .recv_timeout(timeout)?
        else {
            return Ok(None);
        };
        if let Some(snapshot) = frame.snapshot {
            self.state = Some(snapshot.into());
        }
        let Some(event) = frame.event else {
            if frame.stream == "gone" {
                self.attached.take();
                self.finished = self
                    .state
                    .as_ref()
                    .is_some_and(|state| state.status == "stopped");
            }
            return Ok(None);
        };
        let event = canonical_event(event);
        self.seen = self.seen.max(event.seq);
        for handler in &mut self.event_handlers {
            handler(&event);
        }
        for handler in &mut self.log_handlers {
            handler(&event);
        }
        Ok(Some(event))
    }

    /// An iterator over replayed and live semantic Events.
    pub fn events(&mut self) -> Events<'_> {
        Events { chat: self }
    }

    /// All persisted events.
    pub fn history(&self) -> Result<Vec<Event>> {
        self.history_since(0)
    }

    /// Persisted events beginning at `since`.
    pub fn history_since(&self, since: i64) -> Result<Vec<Event>> {
        Ok(self
            .client
            .events(&self.session_id, since)?
            .into_iter()
            .map(canonical_event)
            .collect())
    }

    /// Refresh and return the daemon-owned state.
    pub fn refresh(&mut self) -> Result<ChatState> {
        if self.attached.is_none() {
            self.start_since(self.seen + 1)?;
        }
        let (state, listeners) = self
            .attached
            .as_ref()
            .expect("start populated the attached session")
            .status()?;
        self.listeners = listeners;
        let state = ChatState::from(state);
        self.state = Some(state.clone());
        Ok(state)
    }

    pub fn state(&self) -> Option<&ChatState> {
        self.state.as_ref()
    }

    pub fn status(&self) -> &str {
        self.state
            .as_ref()
            .map_or("idle", |state| state.status.as_str())
    }

    pub fn idle(&self) -> bool {
        self.state.as_ref().is_some_and(ChatState::idle)
    }

    pub fn provider(&self) -> &str {
        self.state
            .as_ref()
            .map_or("", |state| state.provider.as_str())
    }

    pub fn model(&self) -> &str {
        self.state.as_ref().map_or("", |state| state.model.as_str())
    }

    pub fn effort(&self) -> &str {
        self.state
            .as_ref()
            .map_or("", |state| state.effort.as_str())
    }

    pub fn current_intelligence(&self) -> Option<i64> {
        self.state.as_ref().map(|state| state.intelligence)
    }

    pub fn listeners(&self) -> usize {
        self.listeners
    }

    /// Stop listening while the daemon and provider remain warm.
    pub fn detach(&mut self) -> Result<()> {
        let Some(mut session) = self.attached.take() else {
            return Ok(());
        };
        session.detach()
    }

    /// End the daemon-side chat globally and detach this handle.
    pub fn stop(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        if !self.started {
            self.finished = true;
            return Ok(());
        }
        if let Some(mut session) = self.attached.take() {
            session.stop()?;
        } else {
            self.client.stop(&self.session_id)?;
        }
        self.finished = true;
        if let Some(state) = &mut self.state {
            state.status = "stopped".into();
        }
        Ok(())
    }

    fn change(&mut self, what: &str, value: Value) -> Result<&mut Self> {
        if self.finished {
            return Err(Error::Stopped(format!(
                "session {:?} was stopped; load it again to continue",
                self.session_id
            )));
        }
        if let Some(session) = &self.attached {
            if !session.set(what, value)? {
                return Err(Error::Daemon(format!(
                    "session {:?} did not accept setting {what:?}",
                    self.session_id
                )));
            }
        } else {
            self.pending.push(Setting::new(what, value));
        }
        Ok(self)
    }
}

/// Blocking iterator returned by [`Chat::events`].
pub struct Events<'a> {
    chat: &'a mut Chat,
}

impl Iterator for Events<'_> {
    type Item = Result<Event>;

    fn next(&mut self) -> Option<Self::Item> {
        self.chat.next_event().transpose()
    }
}

fn value_as<T: serde::de::DeserializeOwned>(operation: &str, value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|error| Error::Protocol(format!("bad {operation} reply from daemon: {error}")))
}

fn canonical_event(mut event: Event) -> Event {
    if let Some(intelligence) = event.extra.remove("level") {
        event.extra.entry("intelligence").or_insert(intelligence);
    }
    event
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .map_err(|error| Error::Io(format!("cannot read {}: {error}", path.display())))
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| Error::Io(format!("cannot resolve {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use omni_core::wire::{Frame, Reply, Request};
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    fn snapshot(seq: i64) -> Snapshot {
        Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            intelligence: 7,
            providers: vec!["claude".into()],
            provider: "claude".into(),
            model: "sonnet".into(),
            effort: "high".into(),
            cwd: "/tmp".into(),
            seq,
            in_turn: false,
            queued: 0,
        }
    }

    fn read_request(lines: &mut impl Iterator<Item = std::io::Result<String>>) -> Request {
        serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap()
    }

    fn send_json(stream: &mut UnixStream, value: &impl Serialize) {
        serde_json::to_writer(&mut *stream, value).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn the_high_level_api_is_lazy_event_first_and_uses_intelligence() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let client = Client::from_stream(ours).unwrap();
        let server = thread::spawn(move || {
            let reading = theirs.try_clone().unwrap();
            let mut lines = BufReader::new(reading).lines();
            let mut writing = theirs;

            let open = read_request(&mut lines);
            assert_eq!(open.op, "open");
            assert_eq!(open.session.as_deref(), Some("demo"));
            assert_eq!(open.providers, Some(vec!["claude".to_string()]));
            assert_eq!(open.from, Some(0));
            assert_eq!(
                open.value,
                json!([
                    {"what": "intelligence", "value": 7},
                    {"what": "mcp", "value": false},
                ])
            );
            send_json(
                &mut writing,
                &Reply::ok(
                    open.id,
                    json!({
                        "session": "demo",
                        "replayed": 0,
                        "listeners": 1,
                        "snapshot": snapshot(0),
                    }),
                ),
            );

            let send = read_request(&mut lines);
            assert_eq!(send.op, "send");
            assert_eq!(send.text.as_deref(), Some("hello"));
            send_json(&mut writing, &Reply::ok(send.id, json!({"accepted": true})));
            let mut event = Event::new(Event::TEXT).saying("world");
            event.session = "demo".into();
            event.provider = "claude".into();
            event.seq = 1;
            send_json(&mut writing, &Frame::event("demo", event, snapshot(1)));

            let status = read_request(&mut lines);
            assert_eq!(status.op, "status");
            send_json(
                &mut writing,
                &Reply::ok(status.id, json!({"snapshot": snapshot(1), "listeners": 1})),
            );

            let detach = read_request(&mut lines);
            assert_eq!(detach.op, "detach");
            send_json(
                &mut writing,
                &Reply::ok(detach.id, json!({"detached": "demo"})),
            );
        });

        let inference = Inference::from_client(client);
        let mut chat = inference.load_or_create_session("demo", Some(vec!["claude".into()]));
        let handled = Arc::new(AtomicUsize::new(0));
        let for_handler = Arc::clone(&handled);
        chat.on_event(move |event| {
            assert_eq!(event.event_type, Event::TEXT);
            for_handler.fetch_add(1, Ordering::SeqCst);
        });
        let logged = Arc::new(AtomicUsize::new(0));
        let for_log = Arc::clone(&logged);
        chat.logs(move |event| {
            assert_eq!(event.event_type, Event::TEXT);
            for_log.fetch_add(1, Ordering::SeqCst);
        });

        chat.intelligence(7)
            .unwrap()
            .disable_mcp()
            .unwrap()
            .start()
            .unwrap();
        assert_eq!(chat.current_intelligence(), Some(7));
        assert_eq!(chat.effort(), "high");
        chat.send("hello").unwrap();
        let event = chat.next_event().unwrap().unwrap();
        assert_eq!(event.event_type, Event::TEXT);
        assert_eq!(event.text, "world");
        assert_eq!(handled.load(Ordering::SeqCst), 1);
        assert_eq!(logged.load(Ordering::SeqCst), 1);
        let state = chat.refresh().unwrap();
        assert_eq!(state.intelligence, 7);
        assert_eq!(serde_json::to_value(state).unwrap()["intelligence"], 7);
        chat.detach().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn chat_state_serializes_intelligence_while_the_legacy_wire_key_survives() {
        let raw = snapshot(3);
        assert_eq!(raw.intelligence, 7);
        let wire = serde_json::to_value(&raw).unwrap();
        assert_eq!(wire["level"], 7);
        assert!(wire.get("intelligence").is_none());

        let state = ChatState::from(raw);
        let public = serde_json::to_value(&state).unwrap();
        assert_eq!(public["intelligence"], 7);
        assert!(public.get("level").is_none());
        let canonical_input: ChatState = serde_json::from_value(json!({
            "session": "demo",
            "status": "waiting",
            "intelligence": 8,
            "providers": [],
            "provider": "",
            "model": "",
            "effort": "",
            "cwd": "/tmp",
            "seq": 0,
            "in_turn": false,
            "queued": 0,
        }))
        .unwrap();
        assert_eq!(canonical_input.intelligence, 8);

        let mut old_input = public;
        let old_intelligence = old_input
            .as_object_mut()
            .unwrap()
            .remove("intelligence")
            .unwrap();
        old_input["level"] = old_intelligence;
        let from_old: ChatState = serde_json::from_value(old_input).unwrap();
        assert_eq!(from_old.intelligence, 7);

        let mut old_event = Event::config("intelligence");
        old_event.extra.insert("level".into(), json!(7));
        let event = canonical_event(old_event);
        assert_eq!(event.extra["intelligence"], 7);
        assert!(!event.extra.contains_key("level"));
    }
}
