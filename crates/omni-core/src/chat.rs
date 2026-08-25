//! One omni session, whichever provider happens to be running it.
//!
//! Everything that matters happens on a single conductor thread: provider
//! output, user messages and settings changes all arrive on one queue and are
//! handled in order. That is why events come out one at a time, in the order
//! things actually happened, and why nothing ever changes mid-turn.
//!
//! The rule the whole design hangs off: **nothing changes mid turn**.
//! Intelligence, providers, prompts and session swaps are queued when you ask,
//! persisted by the conductor, and applied at the next turn boundary.
//!
//! A [`Chat`] is owned by exactly one thread. Everyone else — every attached
//! client, in any language — holds a [`Handle`], which can only post messages
//! to it and read a [`Snapshot`] of where it got to. There is no shared mutable
//! state to get wrong.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::events::{AUTH, CRASH, Event, event_type};
use crate::choose::{Ask, Pick, resolve};
use crate::providers;
use crate::providers::base::{CONFIRMED, CONFIRMED_AS, Config, Delivery, Emit, GENERATION, Runner};
use crate::session::{Meta, PendingMessage, Store};

pub const IDLE: &str = "idle";
pub const WAITING: &str = "waiting";
pub const BUSY: &str = "busy";
pub const STOPPED: &str = "stopped";

const MODEL_EVENTS: &[&str] = &[
    event_type::TEXT,
    event_type::THINKING,
    event_type::TOOL_CALL,
    event_type::TOOL_RESULT,
];

const ACTIVE_PROVIDERS: &str = "active_providers";
/// The 0.4 metadata spelling retained on disk for cold-session compatibility.
const STORED_ASK: &str = "ask";
const SYSTEM_PROMPT: &str = "system_prompt";
const APPEND_SYSTEM_PROMPT: &str = "append_system_prompt";
const SUBAGENTS: &str = "subagents";
const MCP: &str = "mcp";
const AUTOREMOVE: &str = "autoremove";
const CWD: &str = "cwd";
/// Correlates a durable opening event with the pending message it consumed.
/// Kept in `extra` so older readers remain schema-compatible.
const MESSAGE_ID: &str = "message_id";
/// Identifies one concrete runner instance. Unlike the Codex server generation,
/// this is attached by the conductor to every provider, so queued output from a
/// stopped Claude/Google process cannot act on its replacement.
const RUNNER_EPOCH: &str = "_omni_runner_epoch";

/// Errors that end the turn rather than just being reported.
const FATAL: &[&str] = &[AUTH, CRASH];

/// Notices that are true of a provider rather than of a moment. Worth saying
/// when you ask for the thing, and when the conversation arrives somewhere that
/// cannot do it — not every time a process restarts underneath it.
const ONCE: &[&str] = &["unsupported", "approximated"];

/// A setting the next turn boundary will pick up.
#[derive(Debug, Clone)]
pub enum Change {
    Providers(Vec<String>),
    Model(Ask),
    SystemPrompt(String),
    AppendSystemPrompt(String),
    Subagents(bool),
    Mcp(bool),
    Autoremove(bool),
    Cwd(String),
}

/// What the conductor is asked to do. Everything arrives here, in order.
pub enum Wake {
    Launch,
    /// A cold daemon open waits for the one eager launch boundary. Provider
    /// failure is still a normal session event; only storage failure rejects
    /// the open.
    LaunchReady(Sender<Result<(), String>>),
    Send(String),
    /// The daemon waits on this acknowledgement. It means the message is in
    /// Meta, not that a provider has accepted or completed it.
    SendDurable(String, Sender<Result<(), String>>),
    Heard(Box<Event>),
    Set(Change),
    /// Apply an open/set batch in order and acknowledge only after metadata and
    /// its CONFIG events are durable and published.
    Configure(Vec<Change>, Sender<Result<(), String>>),
    Stop,
}

/// Where a chat has got to. Readable from anywhere; written only by the
/// conductor, once per change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: String,
    pub status: String,
    /// What this chat was told to run: a key, a number, or a model by name.
    pub ask: Ask,
    pub providers: Vec<String>,
    /// The provider currently up, if any.
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub cwd: String,
    pub seq: i64,
    pub in_turn: bool,
    pub queued: usize,
}

impl Snapshot {
    /// Waiting, with no turn open. What a polling loop should check.
    ///
    /// A message omni could not deliver leaves `queued` above zero and the chat
    /// idle, which is the truth: it is doing nothing, and the send that failed
    /// was reported as an error. Retrying is the caller's call, not a reason to
    /// spin.
    pub fn idle(&self) -> bool {
        self.status == WAITING && !self.in_turn
    }
}

/// What every attached client holds. Cloneable, and safe to keep after the
/// chat has stopped — posting to a finished chat is a no-op, never a panic.
#[derive(Clone)]
pub struct Handle {
    pub session_id: String,
    inbox: Sender<Wake>,
    state: Arc<RwLock<Snapshot>>,
}

impl Handle {
    pub fn post(&self, wake: Wake) -> bool {
        self.inbox.send(wake).is_ok()
    }

    pub fn send(&self, text: &str) -> bool {
        self.send_durable(text).is_ok()
    }

    pub fn send_durable(&self, text: &str) -> Result<(), String> {
        // Flip to busy before returning, so a caller polling in a loop never
        // sees a false lull between saying something and it being picked up.
        {
            let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
            if state.status == WAITING || state.status == BUSY {
                state.status = BUSY.into();
            }
        }
        let (ack, waited) = channel();
        self.inbox
            .send(Wake::SendDurable(text.to_string(), ack))
            .map_err(|_| format!("session {:?} is no longer running", self.session_id))?;
        waited.recv().unwrap_or_else(|_| {
            Err(format!(
                "session {:?} stopped before accepting the message",
                self.session_id
            ))
        })
    }

    /// Queue a settings batch without waiting. Registry uses this before the
    /// conductor thread starts, preserving open-time order ahead of Launch.
    pub fn configure_deferred(
        &self,
        changes: Vec<Change>,
    ) -> Result<Receiver<Result<(), String>>, String> {
        let (ack, waited) = channel();
        self.inbox
            .send(Wake::Configure(changes, ack))
            .map_err(|_| format!("session {:?} is no longer running", self.session_id))?;
        Ok(waited)
    }

    pub fn launch_deferred(&self) -> Result<Receiver<Result<(), String>>, String> {
        let (ack, waited) = channel();
        self.inbox
            .send(Wake::LaunchReady(ack))
            .map_err(|_| format!("session {:?} is no longer running", self.session_id))?;
        Ok(waited)
    }

    pub fn configure(&self, changes: Vec<Change>) -> Result<(), String> {
        self.configure_deferred(changes)?
            .recv()
            .unwrap_or_else(|_| {
                Err(format!(
                    "session {:?} stopped while saving settings",
                    self.session_id
                ))
            })
    }

    pub fn set(&self, change: Change) -> Result<(), String> {
        self.configure(vec![change])
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.read().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// A provider left running between uses, and the settings it was left with.
struct Parked {
    runner: Box<dyn Runner>,
    epoch: Option<u64>,
    signature: Signature,
    /// How far up the log it had been told when it was set aside.
    synced: i64,
}

type Signature = (String, String, String, Config);

pub struct Chat {
    pub session_id: String,
    store: Store,
    meta: Meta,
    providers: Vec<String>,
    ask: Ask,
    config: Config,
    sink: Emit,
    inbox: Receiver<Wake>,
    post: Sender<Wake>,
    state: Arc<RwLock<Snapshot>>,
    runner: Option<Box<dyn Runner>>,
    /// The concrete runner currently allowed to affect conductor state.
    runner_epoch: Option<u64>,
    next_runner_epoch: u64,
    /// Last omni turn owned by each runner. Retained after replacement so late
    /// output is logged against its origin rather than the successor's turn.
    epoch_turns: BTreeMap<u64, i64>,
    /// A straggler history event is in the log but was not seen by the current
    /// native provider. Its scalar watermark may not cross this sequence yet.
    sync_ceiling: Option<i64>,
    /// Providers this session has used, left hot rather than killed.
    parked: BTreeMap<String, Parked>,
    /// The (provider, model, effort, config) the runner was built with.
    current: Option<Signature>,
    /// Prevents Registry's launch barrier and the standalone Chat fallback
    /// from starting the same provider twice.
    launched_once: bool,
    outbox: VecDeque<PendingMessage>,
    /// Written to an echo-confirming provider, but not consumed there yet.
    awaiting: VecDeque<PendingMessage>,
    /// Mid-turn messages a provider accepted as later native turns.
    next_turns: usize,
    /// Recorded, not yet handed on. See [`Chat::publish`].
    pending: Vec<Event>,
    /// Current omni turn, starting at -1 before the first user message.
    turn: i64,
    /// Whether the current provider activity has a durable opening event.
    turn_recorded: bool,
    in_turn: bool,
    stopping: bool,
    /// Set only when durable state can no longer be trusted. Unlike a normal
    /// stop, this prevents shutdown from trying to write to the failed store.
    terminal: bool,
    /// Opening is intentionally infallible so callers can always get a handle.
    /// A metadata failure found during construction is reported when `run`
    /// starts, through the same terminal path as every later failure.
    startup_error: Option<String>,
    autoremove: bool,
    announced: BTreeSet<String>,
}

impl Chat {
    /// Open a session. `sink` hears every event the chat records, in order.
    pub fn open(session_id: &str, providers: Vec<String>, sink: Emit) -> (Chat, Handle) {
        Self::open_at(session_id, providers, None, sink)
    }

    /// Open a session, offering the opening client's directory for a session
    /// that has never pinned one. A restored session always wins over this
    /// fallback; an explicit [`Change::Cwd`] can still move it afterwards.
    pub fn open_at(
        session_id: &str,
        providers: Vec<String>,
        opening_cwd: Option<&str>,
        sink: Emit,
    ) -> (Chat, Handle) {
        let store = Store::open(session_id);
        let turn = store.turn();
        let mut meta = Meta::open(session_id);
        let mut startup_error = store
            .load_error()
            .map(|error| format!("loading existing session log: {error}"));
        let delivered: BTreeSet<String> = store
            .events(0)
            .into_iter()
            .filter(|event| event.is(event_type::START) || event.is(event_type::INJECTED))
            .filter_map(|event| {
                event
                    .extra
                    .get(MESSAGE_ID)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect();
        if startup_error.is_none() {
            startup_error = meta
                .load_error()
                .map(|error| format!("loading existing session metadata: {error}"));
        }
        if startup_error.is_none() {
            startup_error = meta
                .reconcile(&delivered)
                .err()
                .map(|error| format!("reconciling delivered messages: {error}"));
        }
        /* What this session was last told to run. A number on its own is a
           0.5 session being reopened, and still means the dial. */
        let ask = meta
            .setting(STORED_ASK)
            .or_else(|| meta.get(STORED_ASK))
            .and_then(|value| match value.as_i64() {
                Some(number) => Some(Ask::intelligence(number)),
                None => serde_json::from_value::<Ask>(value.clone()).ok(),
            })
            .unwrap_or_default();
        let providers = meta
            .setting(ACTIVE_PROVIDERS)
            .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
            .unwrap_or(providers);
        // Claude resumes by cwd, so a session that moves directory loses its
        // provider sessions. Pin the directory to the session the first time.
        let mut config = Config::default();
        let cwd = meta
            .setting_str(CWD)
            .or_else(|| meta.get_str(CWD))
            .filter(|cwd| !cwd.is_empty())
            .or_else(|| {
                opening_cwd
                    .filter(|cwd| !cwd.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| config.cwd.clone());
        config = config.at(&cwd);
        config.system_prompt = meta.setting_str(SYSTEM_PROMPT).unwrap_or_default();
        config.append_system_prompt = meta.setting_str(APPEND_SYSTEM_PROMPT).unwrap_or_default();
        let subagents = meta
            .setting(SUBAGENTS)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        config.disable_subagents = !subagents;
        let mcp = meta
            .setting(MCP)
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        config.disable_mcp = !mcp;
        let autoremove = meta
            .setting(AUTOREMOVE)
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let mut initial_settings = serde_json::Map::new();
        initial_settings.insert(ACTIVE_PROVIDERS.into(), json!(providers));
        initial_settings.insert(STORED_ASK.into(), json!(ask));
        initial_settings.insert(SYSTEM_PROMPT.into(), json!(config.system_prompt));
        initial_settings.insert(
            APPEND_SYSTEM_PROMPT.into(),
            json!(config.append_system_prompt),
        );
        initial_settings.insert(SUBAGENTS.into(), json!(subagents));
        initial_settings.insert(MCP.into(), json!(mcp));
        initial_settings.insert(AUTOREMOVE.into(), json!(autoremove));
        initial_settings.insert(CWD.into(), json!(config.cwd));
        if startup_error.is_none() {
            startup_error = meta
                .initialize_settings(initial_settings)
                .err()
                .map(|error| format!("initializing session settings: {error}"));
        }
        let outbox: VecDeque<PendingMessage> = meta.pending().into();
        let queued = outbox.len();
        let (post, inbox) = channel();
        let state = Arc::new(RwLock::new(Snapshot {
            session: session_id.to_string(),
            status: IDLE.into(),
            ask: ask.clone(),
            providers: providers.clone(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: config.cwd.clone(),
            seq: store.seq(),
            in_turn: false,
            queued,
        }));
        let handle = Handle {
            session_id: session_id.to_string(),
            inbox: post.clone(),
            state: state.clone(),
        };
        let chat = Chat {
            session_id: session_id.to_string(),
            store,
            meta,
            providers,
            ask,
            config,
            sink,
            inbox,
            post,
            state,
            runner: None,
            runner_epoch: None,
            next_runner_epoch: 1,
            epoch_turns: BTreeMap::new(),
            sync_ceiling: None,
            parked: BTreeMap::new(),
            current: None,
            launched_once: false,
            outbox,
            awaiting: VecDeque::new(),
            next_turns: 0,
            pending: Vec::new(),
            turn,
            turn_recorded: false,
            in_turn: false,
            stopping: false,
            terminal: false,
            startup_error,
            autoremove,
            announced: BTreeSet::new(),
        };
        (chat, handle)
    }

    /// The one thread that owns this chat's state. Returns when it stops.
    pub fn run(mut self) {
        if let Some(error) = self.startup_error.take() {
            self.persistence_failed("opening durable session state", &error);
            self.publish();
            self.reject_waiters(&format!("session persistence failed: {error}"));
            return;
        }
        self.settle(|state| state.status = WAITING.into());
        let _ = self.post.send(Wake::Launch);
        while let Ok(wake) = self.inbox.recv() {
            match wake {
                Wake::Stop => break,
                Wake::Launch => self.launch_once(),
                Wake::LaunchReady(ack) => {
                    self.launch_once();
                    self.publish();
                    let result = match self.terminal {
                        true => Err("session persistence failed during provider launch".into()),
                        false => Ok(()),
                    };
                    let _ = ack.send(result);
                    if self.terminal {
                        self.reject_waiters("session persistence failed");
                        return;
                    }
                    continue;
                }
                Wake::Send(text) => self.dispatch(text),
                Wake::SendDurable(text, ack) => {
                    let result = self.enqueue(text);
                    // Make the accepted/terminal snapshot visible before the
                    // waiting daemon request is released.
                    self.publish();
                    let accepted = result.is_ok();
                    let _ = ack.send(result);
                    if accepted && !self.terminal {
                        // Provider work begins only after the local durability
                        // acknowledgement has been sent.
                        self.dispatch_queued();
                        self.publish();
                    }
                    if self.terminal {
                        self.reject_waiters("session persistence failed");
                        return;
                    }
                    continue;
                }
                Wake::Heard(event) => self.absorb(*event),
                Wake::Set(change) => {
                    let _ = self.apply(change);
                }
                Wake::Configure(changes, ack) => {
                    let result = self.apply_all(changes);
                    // CONFIG events must be published before an open can attach
                    // and replay; otherwise it could receive the same event in
                    // both the replay and the live stream.
                    self.publish();
                    let _ = ack.send(result);
                    if self.terminal {
                        self.reject_waiters("session persistence failed");
                        return;
                    }
                    continue;
                }
            }
            self.publish();
            if self.terminal {
                self.reject_waiters("session persistence failed");
                return;
            }
        }
        self.shutdown();
        self.publish();
    }

    /// Where a runner's events go: back onto this chat's own queue.
    fn ear(&self, epoch: u64) -> Emit {
        let post = self.post.clone();
        Arc::new(move |mut event| {
            event.extra.insert(RUNNER_EPOCH.into(), json!(epoch));
            let _ = post.send(Wake::Heard(Box::new(event)));
        })
    }

    // ------------------------------------------------------------------ setup

    fn apply_all(&mut self, changes: Vec<Change>) -> Result<(), String> {
        for change in changes {
            self.apply(change)?;
        }
        Ok(())
    }

    fn apply(&mut self, change: Change) -> Result<(), String> {
        let (what, extra) = match change {
            Change::Providers(providers) => {
                self.save_setting(
                    ACTIVE_PROVIDERS,
                    json!(providers),
                    "saving active-provider metadata",
                )?;
                self.providers = providers;
                ("providers", json!({"providers": self.providers}))
            }
            Change::Model(ask) => {
                self.save_setting(STORED_ASK, json!(ask), "saving model metadata")?;
                self.ask = ask.clone();
                ("model", json!({"ask": ask}))
            }
            Change::SystemPrompt(text) => {
                self.save_setting(SYSTEM_PROMPT, json!(text), "saving system-prompt metadata")?;
                let chars = text.chars().count();
                self.config.system_prompt = text;
                ("system_prompt", json!({"chars": chars}))
            }
            Change::AppendSystemPrompt(text) => {
                self.save_setting(
                    APPEND_SYSTEM_PROMPT,
                    json!(text),
                    "saving appended-system-prompt metadata",
                )?;
                let chars = text.chars().count();
                self.config.append_system_prompt = text;
                ("append_system_prompt", json!({"chars": chars}))
            }
            Change::Subagents(on) => {
                self.save_setting(SUBAGENTS, json!(on), "saving subagent metadata")?;
                self.config.disable_subagents = !on;
                // Whether a provider can honour this may have changed.
                self.announced.clear();
                ("subagents", json!({"on": on}))
            }
            Change::Mcp(on) => {
                self.save_setting(MCP, json!(on), "saving MCP metadata")?;
                self.config.disable_mcp = !on;
                self.announced.clear();
                ("mcp", json!({"on": on}))
            }
            Change::Autoremove(on) => {
                self.save_setting(AUTOREMOVE, json!(on), "saving provider-autoremove metadata")?;
                self.autoremove = on;
                ("autoremove_unauthenticated", json!({"on": on}))
            }
            Change::Cwd(path) => {
                let config = self.config.clone().at(&path);
                self.save_setting(CWD, json!(config.cwd), "saving working-directory metadata")?;
                self.config = config;
                ("cwd", json!({"cwd": self.config.cwd}))
            }
        };
        let extra = extra.as_object().cloned().unwrap_or_default();
        if self.record(Event::config(what).extras(extra)).is_none() {
            return Err("session persistence failed while recording a setting".into());
        }
        self.refresh();
        Ok(())
    }

    fn save_setting(
        &mut self,
        key: &str,
        value: serde_json::Value,
        action: &str,
    ) -> Result<(), String> {
        match self.meta.set_setting(key, value) {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = format!("{action}: {error}");
                self.persistence_failed(action, &error);
                Err(message)
            }
        }
    }

    // -------------------------------------------------------------- conductor

    fn launch_once(&mut self) {
        if self.launched_once {
            return;
        }
        self.launched_once = true;
        let ready = self.runner.as_ref().is_some_and(|runner| runner.alive()) && !self.stale();
        if (ready || self.attempt("starting up")) && !self.terminal {
            self.flush();
        }
    }

    fn absorb(&mut self, mut event: Event) {
        let event_epoch = event
            .extra
            .get(RUNNER_EPOCH)
            .and_then(|value| value.as_u64());
        if event.turn < 0 {
            if let Some(origin) = event_epoch.and_then(|epoch| self.epoch_turns.get(&epoch)) {
                event.turn = *origin;
            }
        }
        let straggler = self.straggler(&event);
        // Runner/server generations are queue provenance, not public history.
        // Work out whether this is stale and recover its origin turn first.
        event.extra.remove(GENERATION);
        event.extra.remove(RUNNER_EPOCH);
        if straggler {
            // The internal epoch is intentionally not durable, but whether an
            // event arrived from a replaced runner matters to future seeding.
            // In particular, a same-provider replacement may have a different
            // native session and must not filter this event as already known.
            event.extra.insert("late".into(), json!(true));
        }
        if self.repeated(&event) {
            return;
        }
        if event.is(CONFIRMED) {
            if !straggler {
                self.confirm(event);
            }
            return;
        }
        let Some(event) = self.record(event) else {
            return;
        };
        if straggler {
            if event.is_history() && self.runner.is_some() {
                let ceiling = event.seq - 1;
                self.sync_ceiling = Some(
                    self.sync_ceiling
                        .map_or(ceiling, |existing| existing.min(ceiling)),
                );
            }
            return; // still logged, but there is nothing left to react to
        }
        if self.stopping {
            // A real END emitted while interrupting wins over the synthetic
            // fallback in shutdown. Clear the durable turn here, but never
            // launch, retune, or flush more work while stopping.
            if event.is(event_type::END) {
                self.in_turn = false;
                self.turn_recorded = false;
                self.next_turns = 0;
            }
            return;
        }
        if MODEL_EVENTS.contains(&event.event_type.as_str()) {
            self.settle(|state| state.status = BUSY.into());
        } else if event.is(event_type::END) {
            self.in_turn = false;
            self.turn_recorded = false;
            self.finish_turn();
        } else if event.is(event_type::ERROR)
            && FATAL.contains(&event.kind.as_str())
            && event
                .extra
                .get("willRetry")
                .and_then(|value| value.as_bool())
                != Some(true)
        {
            self.fatal(&event);
        }
    }

    /// Turn a provider-private delivery acknowledgement into the one durable
    /// user event it confirms. Exact FIFO matching handles duplicate messages
    /// and rejects Claude's unrelated control-channel replay chatter.
    fn confirm(&mut self, mut event: Event) {
        let Some(message) = self.awaiting.front().cloned() else {
            return;
        };
        if message.text != event.text {
            return;
        }
        let Some(opening) = event
            .extra
            .remove(CONFIRMED_AS)
            .and_then(|value| value.as_str().map(str::to_string))
        else {
            return;
        };
        if !matches!(opening.as_str(), event_type::START | event_type::INJECTED) {
            return;
        }
        event.event_type = opening;
        event
            .extra
            .insert(MESSAGE_ID.into(), json!(message.id.clone()));
        if event.is(event_type::START) {
            event.turn = self.turn + 1;
        } else {
            event.extra.insert("landed".into(), json!("same_turn"));
        }
        let Some(event) = self.record(event) else {
            return;
        };
        self.turn = self.turn.max(event.turn);
        self.remember_runner_turn();
        if !self.complete_message(&message.id) {
            return;
        }
        self.awaiting.pop_front();
        self.turn_recorded = true;
        self.in_turn = true;
        self.settle(|state| state.status = BUSY.into());
    }

    /// Has this provider already told us it cannot do this?
    ///
    /// Forgotten when the setting changes and when the conversation moves to
    /// another provider, which are the two moments the answer can differ.
    fn repeated(&mut self, event: &Event) -> bool {
        if !event.is(event_type::CONFIG) || !ONCE.contains(&event.text.as_str()) {
            return false;
        }
        // The serialised form, not the fields: `extra` holds lists, and two
        // notices differ only by what is in them.
        let said = format!(
            "{}\u{1}{}\u{1}{}",
            event.text,
            event.provider,
            json!(event.extra)
        );
        !self.announced.insert(said)
    }

    /// Did this come from a provider omni has already moved on from?
    ///
    /// A dying CLI reports the failure and the end of its turn in one breath.
    /// By the time the second one is handled there may be a new provider up,
    /// and applying it there would close a turn that is still open — or take
    /// down the runner that has just taken over.
    fn straggler(&self, event: &Event) -> bool {
        if let Some(epoch) = event
            .extra
            .get(RUNNER_EPOCH)
            .and_then(|value| value.as_u64())
        {
            if self.runner_epoch != Some(epoch) {
                return true;
            }
        }
        match &self.runner {
            Some(runner) => {
                if !event.provider.is_empty() && event.provider != runner.name() {
                    return true;
                }
                let event_generation = event.extra.get(GENERATION).and_then(|value| value.as_u64());
                match (event_generation, runner.generation()) {
                    (Some(event), Some(current)) => event != current,
                    _ => false,
                }
            }
            None => false,
        }
    }

    /// An error that ends the turn: a crash, or a login that has gone.
    fn fatal(&mut self, event: &Event) {
        if event.kind == AUTH {
            self.unauthenticated(event);
        } else {
            // The provider died. Close the turn honestly and let the next send
            // retry.
            self.close_turn(&event.provider, json!({"crashed": true}));
        }
    }

    /// A provider lost its login mid-run. Take it off the dial and move on.
    ///
    /// The turn is over either way — an unauthenticated CLI cannot finish it.
    /// What differs is the next one: by default that provider is dropped from
    /// this chat and the same intelligence level is resolved again over the
    /// providers that are left, so the conversation carries on somewhere else.
    fn unauthenticated(&mut self, event: &Event) {
        let name = match event.provider.is_empty() {
            false => event.provider.clone(),
            true => self
                .runner
                .as_ref()
                .map(|r| r.name().to_string())
                .unwrap_or_default(),
        };
        if !self.close_turn(&name, json!({"unauthenticated": name})) {
            return;
        }
        if !self.autoremove || !self.providers.contains(&name) {
            return;
        }
        let remaining: Vec<String> = self
            .providers
            .iter()
            .filter(|provider| *provider != &name)
            .cloned()
            .collect();
        if self
            .save_setting(
                ACTIVE_PROVIDERS,
                json!(remaining),
                "saving an automatically-removed provider",
            )
            .is_err()
        {
            return;
        }
        self.providers = remaining;
        // A login can come back; ask the CLI again next time.
        providers::forget(&name);
        // Whatever it had parked is no longer usable to anyone.
        if let Some(mut stale) = self.parked.remove(&name) {
            stale.runner.stop();
        }
        if self
            .record(
                Event::config("provider_removed")
                    .from(&name)
                    .with("why", "unauthenticated")
                    .with("left", json!(self.providers)),
            )
            .is_none()
        {
            return;
        }
        self.refresh();
        if self.providers.is_empty() {
            self.blocked("every provider is unauthenticated; log one back in and send again");
            return;
        }
        if self.attempt("provider lost its login") {
            self.flush();
        }
    }

    /// Put the runner down and end the turn it was in the middle of.
    fn close_turn(&mut self, provider: &str, extra: serde_json::Value) -> bool {
        if let Some(mut runner) = self.runner.take() {
            runner.stop();
        }
        self.runner_epoch = None;
        // A pipe write that Claude never echoed was never delivered. Put it
        // back in front of anything newer so a later provider can retry it.
        while let Some(text) = self.awaiting.pop_back() {
            self.outbox.push_front(text);
        }
        self.next_turns = 0;
        if self.in_turn && self.turn_recorded {
            self.in_turn = false;
            self.turn_recorded = false;
            let extra = extra.as_object().cloned().unwrap_or_default();
            if self
                .record(Event::new(event_type::END).from(provider).extras(extra))
                .is_none()
            {
                return false;
            }
        }
        self.in_turn = false;
        self.settle(|state| state.status = WAITING.into());
        true
    }

    fn finish_turn(&mut self) {
        // agy turns a mid-flight write into a later native turn. Its first END
        // is a boundary in the stream, but not an idle moment for this chat.
        if self.next_turns > 0 {
            self.next_turns -= 1;
            self.in_turn = true;
            self.turn_recorded = true;
            self.settle(|state| state.status = BUSY.into());
            return;
        }
        if let Some(runner) = &self.runner {
            let name = runner.name().to_string();
            // A late history event is a real hole in this native session, not
            // merely a one-turn anomaly. Keep the ceiling until this runner is
            // replaced/adopted through a catch-up path; a later native turn
            // must not leap the scalar watermark over history it never saw.
            let synced = self.sync_ceiling.unwrap_or_else(|| self.store.seq());
            if let Err(error) = self.meta.mark_synced(&name, synced) {
                self.persistence_failed("saving a provider watermark", &error);
                return;
            }
        }
        // Claude may already have a later line on its pipe whose replay echo
        // follows this END. Keep polling clients out of a false idle gap.
        if !self.awaiting.is_empty() {
            self.in_turn = true;
            self.settle(|state| state.status = BUSY.into());
            return;
        }
        if self.stale() {
            // Rebuilding can include several durable metadata rewrites. Do not
            // expose a false idle window while the requested provider is still
            // being brought into place.
            self.settle(|state| state.status = BUSY.into());
            if !self.attempt("settings changed") {
                return;
            }
        }
        self.flush();
        if !self.in_turn && self.outbox.is_empty() && self.awaiting.is_empty() && !self.terminal {
            self.settle(|state| state.status = WAITING.into());
        }
    }

    /// Put a message in the transactional outbox. This is the local durability
    /// boundary acknowledged by the daemon; no provider work happens here.
    fn enqueue(&mut self, text: String) -> Result<(), String> {
        match self.meta.enqueue(&text) {
            Ok(message) => {
                self.outbox.push_back(message);
                self.settle(|state| state.status = BUSY.into());
                Ok(())
            }
            Err(error) => {
                let message = format!("saving an accepted user message: {error}");
                self.persistence_failed("saving an accepted user message", &error);
                Err(message)
            }
        }
    }

    /// Queue a message, make sure something can carry it, then hand it over.
    fn dispatch(&mut self, text: String) {
        if self.enqueue(text).is_err() {
            return;
        }
        self.dispatch_queued();
    }

    fn dispatch_queued(&mut self) {
        let ready = if !self.runner.as_ref().is_some_and(|runner| runner.alive()) {
            self.attempt("no provider is running")
        } else if !self.in_turn && self.stale() {
            self.attempt("settings changed") // between turns, so it can land now
        } else {
            true
        };
        if ready {
            self.flush();
        }
    }

    /// Hand queued messages over — never to a runner we are about to replace.
    ///
    /// A message is recorded only once the runner has taken it, so it is either
    /// seeded into a provider or sent to one, and never both, and a send that
    /// fails leaves the message queued rather than half-delivered.
    fn flush(&mut self) {
        if !self.runner.as_ref().is_some_and(|runner| runner.alive()) || self.stale() {
            if !self.outbox.is_empty() && !self.runner.as_ref().is_some_and(|r| r.alive()) {
                let queued = self.outbox.len();
                self.blocked(&format!(
                    "nothing is running; {queued} message(s) still queued"
                ));
            }
            return;
        }
        while let Some(message) = self.outbox.front().cloned() {
            let was_in_turn = self.in_turn;
            let sent = match self.runner.as_mut() {
                Some(runner) => runner.send(&message.text),
                None => return,
            };
            let delivery = match sent {
                Ok(delivery) => delivery,
                Err(why) => {
                    let name = self
                        .runner
                        .as_ref()
                        .map(|r| r.name().to_string())
                        .unwrap_or_default();
                    self.blocked(&format!("{name} would not take the message: {why}"));
                    return;
                }
            };
            if delivery == Delivery::Echoed {
                self.awaiting.push_back(message);
                self.outbox.pop_front();
                // Busy immediately, while START/INJECTED waits for the replay
                // acknowledgement that will make it durable.
                self.in_turn = true;
                self.settle(|state| state.status = BUSY.into());
                continue;
            }
            let opening = if was_in_turn {
                event_type::INJECTED
            } else {
                event_type::START
            };
            let mut event = Event::new(opening)
                .saying(message.text.clone())
                .with(MESSAGE_ID, message.id.clone());
            if event.is(event_type::START) {
                event.turn = self.turn + 1;
            } else {
                let landed = match delivery {
                    Delivery::Immediate => "same_turn",
                    Delivery::NextTurn => "next_turn",
                    Delivery::Echoed => unreachable!("handled above"),
                };
                event.extra.insert("landed".into(), json!(landed));
            }
            let Some(event) = self.record(event) else {
                return;
            };
            self.turn = self.turn.max(event.turn);
            self.remember_runner_turn();
            if delivery == Delivery::NextTurn && was_in_turn {
                self.next_turns += 1;
            }
            if !self.complete_message(&message.id) {
                return;
            }
            self.outbox.pop_front();
            self.turn_recorded = true;
            self.in_turn = true;
            self.settle(|state| state.status = BUSY.into());
        }
    }

    fn complete_message(&mut self, id: &str) -> bool {
        match self.meta.complete(id) {
            Ok(()) => true,
            Err(error) => {
                self.persistence_failed("retiring a delivered user message", &error);
                false
            }
        }
    }

    fn remember_runner_turn(&mut self) {
        if let Some(epoch) = self.runner_epoch {
            self.epoch_turns.insert(epoch, self.turn);
        }
    }

    /// Bring a provider up, and survive it refusing to come up.
    fn attempt(&mut self, why: &str) -> bool {
        match self.rebuild() {
            Ok(()) => true,
            Err(err) => {
                if !self.terminal {
                    self.blocked(&format!("could not start a provider ({why}): {err}"));
                }
                false
            }
        }
    }

    /// Say what went wrong and go back to waiting, rather than hanging in 'busy'.
    ///
    /// Queued messages stay queued: the next send retries the whole thing.
    fn blocked(&mut self, why: &str) {
        if self.record(Event::failure(CRASH, why)).is_some() {
            self.settle(|state| state.status = WAITING.into());
        }
    }

    // ---------------------------------------------------------------- runners

    fn rung(&self) -> Result<Pick, String> {
        resolve(&self.ask, &self.providers).map_err(|err| err.to_string())
    }

    fn signature(&self, rung: &Pick) -> Signature {
        (
            rung.provider.clone(),
            rung.model.clone(),
            rung.effort.clone(),
            self.config.clone(),
        )
    }

    /// Has anything been asked for that the running CLI cannot honour?
    ///
    /// A dial we can no longer resolve counts. Being told to use providers omni
    /// has no dial for is an instruction, and quietly carrying on with the CLI
    /// that happens to be up is not following it — the next boundary reports
    /// instead.
    fn stale(&self) -> bool {
        let Some(current) = &self.current else {
            return false;
        };
        match self.rung() {
            Ok(rung) => *current != self.signature(&rung),
            Err(_) => true,
        }
    }

    /// Is this only a model or effort change on the provider already running?
    fn tunable(&self, rung: &Pick) -> bool {
        let Some((provider, model, effort, config)) = &self.current else {
            return false;
        };
        if !self.runner.as_ref().is_some_and(|runner| runner.alive()) {
            return false;
        }
        let wanted = self.signature(rung);
        *provider == wanted.0 && *config == wanted.3 && (model, effort) != (&wanted.1, &wanted.2)
    }

    /// Bring up the provider the current settings ask for. Turn boundaries only.
    fn rebuild(&mut self) -> Result<(), String> {
        let rung = self.rung()?;
        if self.tunable(&rung) {
            let took = self
                .runner
                .as_mut()
                .is_some_and(|runner| runner.retune(&rung.model, &rung.effort));
            if took {
                self.current = Some(self.signature(&rung));
                if self
                    .record(
                        Event::config("retune")
                            .from(&rung.provider)
                            .about(&rung.model)
                            .with("effort", rung.effort.clone())
                            .with("fast", rung.fast),
                    )
                    .is_none()
                {
                    return Err("session persistence failed".into());
                }
                self.refresh();
                return Ok(());
            }
        }
        // `current` outlives the runner, so a provider that crashed or lost its
        // login is still named as the one we came from.
        let previous = match (&self.runner, &self.current) {
            (Some(runner), _) => runner.name().to_string(),
            (None, Some((provider, ..))) => provider.clone(),
            _ => String::new(),
        };
        if let Some(runner) = self.runner.take() {
            // Its `synced` mark stays where the last turn left it: anything
            // recorded since then is exactly what it has to be told on the way
            // back. Left running rather than killed, so coming back is free.
            if !self.park(runner) {
                return Err("session persistence failed".into());
            }
        }
        if previous != rung.provider {
            // A new provider gets to say what it cannot do.
            self.announced.clear();
            if !previous.is_empty()
                && self
                    .record(
                        Event::new(event_type::SWITCH_PROVIDER)
                            .from(&rung.provider)
                            .about(&rung.model)
                            .with("from", previous)
                            .with("to", rung.provider.clone())
                            .with("fast", rung.fast),
                    )
                    .is_none()
            {
                return Err("session persistence failed".into());
            }
        }
        self.launch(&rung)
    }

    /// Set a provider aside still running, so coming back costs nothing.
    fn park(&mut self, mut runner: Box<dyn Runner>) -> bool {
        let name = runner.name().to_string();
        let epoch = self.runner_epoch.take();
        let native = runner.native_id();
        if !native.is_empty() {
            if let Err(error) = self.meta.bind(&name, &native) {
                runner.stop();
                self.persistence_failed("saving a provider session id", &error);
                return false;
            }
        }
        let synced = self.meta.native(&name).1;
        if !runner.parkable() || self.current.is_none() {
            runner.stop();
            return true;
        }
        let signature = self.current.clone().expect("checked just above");
        self.parked.insert(
            name,
            Parked {
                runner,
                epoch,
                signature,
                synced,
            },
        );
        true
    }

    /// Take a parked provider back, if it can be brought up to date in place.
    ///
    /// A parked runner has heard nothing since it was set aside. Some providers
    /// can be told (codex accepts items on a live thread, agy folds text into
    /// the next message); Claude cannot, because it read its history from a
    /// file once. Whoever cannot is stopped and launched again, which is always
    /// correct — just not free.
    fn unpark(&mut self, provider: &str) -> Option<(Box<dyn Runner>, Option<u64>)> {
        let mut parked = self.parked.remove(provider)?;
        if !parked.runner.alive() {
            return None;
        }
        let missed = self.history_for(provider, parked.synced + 1, true);
        if parked.runner.catch_up(&missed) {
            if parked.runner.seeded() {
                if let Err(error) = self.meta.mark_synced(provider, self.store.seq()) {
                    parked.runner.stop();
                    self.persistence_failed("saving a provider watermark", &error);
                    return None;
                }
            }
            return Some((parked.runner, parked.epoch));
        }
        parked.runner.stop();
        None
    }

    fn launch(&mut self, rung: &Pick) -> Result<(), String> {
        let (native_id, synced) = self.meta.native(&rung.provider);
        let wanted = self.signature(rung);

        // Already up and reachable? Then there is nothing to launch.
        if let Some((mut kept, epoch, tuning_matches)) = self.take_parked(&rung.provider, &wanted) {
            if tuning_matches || kept.retune(&rung.model, &rung.effort) {
                return self.adopt(kept, epoch, rung, false);
            }
            kept.stop();
        }
        if self.terminal {
            return Err("session persistence failed".into());
        }

        let mut fresh = native_id.is_empty();
        let config = Config {
            model: rung.model.clone(),
            effort: rung.effort.clone(),
            fast: rung.fast,
            ..self.config.clone()
        };
        let (mut runner, mut epoch) = self.bring_up(rung, &config, &native_id, synced + 1)?;
        if !native_id.is_empty() && runner.native_id() != native_id {
            // The provider no longer knows that session. Rather than carry on
            // with half a conversation, start clean and tell it everything.
            runner.stop();
            if self
                .record(
                    Event::config("reseed")
                        .from(&rung.provider)
                        .with("lost", native_id.clone())
                        .with("now", runner.native_id()),
                )
                .is_none()
            {
                return Err("session persistence failed".into());
            }
            (runner, epoch) = self.bring_up(rung, &config, "", 0)?;
            fresh = true;
        }
        self.adopt(runner, Some(epoch), rung, fresh)
    }

    /// A parked runner whose settings still match what is wanted.
    fn take_parked(
        &mut self,
        provider: &str,
        wanted: &Signature,
    ) -> Option<(Box<dyn Runner>, Option<u64>, bool)> {
        let parked = self.parked.get(provider)?;
        if parked.signature.3 != wanted.3 {
            if let Some(mut stale) = self.parked.remove(provider) {
                stale.runner.stop(); // its config is out of date; it cannot be reused
            }
            return None;
        }
        let tuning_matches = parked.signature.1 == wanted.1 && parked.signature.2 == wanted.2;
        self.unpark(provider)
            .map(|(runner, epoch)| (runner, epoch, tuning_matches))
    }

    fn adopt(
        &mut self,
        runner: Box<dyn Runner>,
        epoch: Option<u64>,
        rung: &Pick,
        fresh: bool,
    ) -> Result<(), String> {
        let native = runner.native_id();
        let seeded = runner.seeded();
        self.runner = Some(runner);
        self.runner_epoch = epoch;
        self.sync_ceiling = None;
        self.remember_runner_turn();
        self.current = Some(self.signature(rung));
        if let Err(error) = self.meta.bind(&rung.provider, &native) {
            self.persistence_failed("saving a provider session id", &error);
            return Err("session persistence failed".into());
        }
        if seeded {
            if let Err(error) = self.meta.mark_synced(&rung.provider, self.store.seq()) {
                self.persistence_failed("saving a provider watermark", &error);
                return Err("session persistence failed".into());
            }
        }
        if self
            .record(
                Event::config("launch")
                    .from(&rung.provider)
                    .about(&rung.model)
                    .with("effort", rung.effort.clone())
                    .with("fast", rung.fast)
                    .with("native", native.clone()),
            )
            .is_none()
        {
            return Err("session persistence failed".into());
        }
        if fresh
            && self
                .record(
                    Event::new(event_type::NEW_SESSION)
                        .from(&rung.provider)
                        .about(&rung.model)
                        .with("native", native),
                )
                .is_none()
        {
            return Err("session persistence failed".into());
        }
        self.refresh();
        Ok(())
    }

    fn bring_up(
        &mut self,
        rung: &Pick,
        config: &Config,
        native_id: &str,
        since: i64,
    ) -> Result<(Box<dyn Runner>, u64), String> {
        let epoch = self.next_runner_epoch;
        self.next_runner_epoch = self.next_runner_epoch.saturating_add(1);
        self.epoch_turns.insert(epoch, self.turn);
        let mut runner =
            providers::runner(&rung.provider, &self.session_id, config, self.ear(epoch))
                .ok_or_else(|| format!("unknown provider {:?}", rung.provider))?;
        let history = self.history_for(&rung.provider, since, !native_id.is_empty());
        runner.start(native_id, &history)?;
        Ok((runner, epoch))
    }

    /// History a continuing native session genuinely missed.
    ///
    /// A watermark can deliberately stop before a late foreign event. Events
    /// after that hole which bear this provider's own name are already in its
    /// native conversation, so replaying them would duplicate user/model
    /// messages. A fresh native id has no such knowledge and receives all of it.
    fn history_for(&self, provider: &str, since: i64, continuing_native: bool) -> Vec<Event> {
        self.store
            .history(since)
            .into_iter()
            .filter(|event| {
                !continuing_native
                    || event.provider != provider
                    || event.extra.get("late").and_then(|value| value.as_bool()) == Some(true)
            })
            .collect()
    }

    fn shutdown(&mut self) {
        if self.terminal {
            return;
        }
        self.stopping = true;
        if let Some(mut runner) = self.runner.take() {
            if self.in_turn {
                runner.interrupt(); // ask nicely before closing the pipe
            }
            let name = runner.name().to_string();
            let native = runner.native_id();
            if !native.is_empty() {
                if let Err(error) = self.meta.bind(&name, &native) {
                    runner.stop();
                    self.persistence_failed("saving a provider session id", &error);
                    return;
                }
            }
            runner.stop();
        }
        for (_, mut parked) in std::mem::take(&mut self.parked) {
            parked.runner.stop();
        }
        if !self.drain() {
            return;
        }
        self.runner_epoch = None;
        if self.in_turn && self.turn_recorded {
            let (provider, model) = self
                .current
                .as_ref()
                .map(|(provider, model, ..)| (provider.clone(), model.clone()))
                .unwrap_or_default();
            if self
                .record(
                    Event::new(event_type::END)
                        .from(&provider)
                        .about(&model)
                        .with("interrupted", true)
                        .with("stopped", true),
                )
                .is_none()
            {
                return;
            }
        }
        self.in_turn = false;
        self.turn_recorded = false;
        self.next_turns = 0;
        // An echo-confirming write without a durable opening event remains an
        // acknowledged pending message. Put it back in FIFO order for the
        // snapshot and the next cold reopen; never erase it on stop.
        while let Some(message) = self.awaiting.pop_back() {
            self.outbox.push_front(message);
        }
        if self.record(Event::config("stop")).is_some() {
            self.settle(|state| state.status = STOPPED.into());
        }
    }

    /// Whatever the provider said on its way out still belongs in the log.
    fn drain(&mut self) -> bool {
        loop {
            match self.inbox.try_recv() {
                Ok(Wake::Heard(event)) => {
                    self.absorb(*event);
                    if self.terminal {
                        return false;
                    }
                }
                Ok(Wake::SendDurable(_, ack)) => {
                    let _ = ack.send(Err("session stopped before accepting the message".into()));
                }
                Ok(Wake::Configure(_, ack)) => {
                    let _ = ack.send(Err("session stopped before saving settings".into()));
                }
                Ok(Wake::LaunchReady(ack)) => {
                    let _ = ack.send(Err("session stopped before provider launch".into()));
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return true,
            }
        }
    }

    /// Release every synchronous caller if the conductor terminates before its
    /// queued command is reached. Otherwise a racing daemon request could wait
    /// forever after a persistence failure or stop.
    fn reject_waiters(&mut self, why: &str) {
        while let Ok(wake) = self.inbox.try_recv() {
            match wake {
                Wake::SendDurable(_, ack) | Wake::Configure(_, ack) | Wake::LaunchReady(ack) => {
                    let _ = ack.send(Err(why.to_string()));
                }
                _ => {}
            }
        }
    }

    // ----------------------------------------------------------------- record

    /// Persist, then tell everyone. The session file is the event log.
    fn record(&mut self, mut event: Event) -> Option<Event> {
        if event.session.is_empty() {
            event.session = self.session_id.clone();
        }
        if event.provider.is_empty() {
            if let Some(runner) = &self.runner {
                event.provider = runner.name().to_string();
                if event.model.is_empty() {
                    event.model = self
                        .current
                        .as_ref()
                        .map(|(_, model, ..)| model.clone())
                        .unwrap_or_default();
                }
            }
        }
        if event.turn < 0 {
            event.turn = self.turn;
        }
        let event = match self.store.append(event) {
            Ok(event) => event,
            Err(error) => {
                self.persistence_failed("appending the session event log", &error);
                return None;
            }
        };
        let seq = event.seq;
        self.settle(|state| state.seq = seq);
        self.pending.push(event.clone());
        Some(event)
    }

    /// Storage is the source of truth, so losing it is a session-ending
    /// condition. This notice is deliberately not passed back through
    /// `record`: the operation that failed may be exactly the one it needs.
    /// Its negative sequence also keeps listeners from treating it as durable
    /// or advancing their replay cursor.
    fn persistence_failed(&mut self, action: &str, error: &dyn std::fmt::Display) {
        if self.terminal {
            return;
        }
        self.terminal = true;
        self.stopping = true;

        let (provider, model) = self
            .current
            .as_ref()
            .map(|(provider, model, ..)| (provider.clone(), model.clone()))
            .unwrap_or_default();
        if let Some(mut runner) = self.runner.take() {
            if self.in_turn {
                runner.interrupt();
            }
            runner.stop();
        }
        for (_, mut parked) in std::mem::take(&mut self.parked) {
            parked.runner.stop();
        }
        self.current = None;
        self.runner_epoch = None;
        self.in_turn = false;
        self.turn_recorded = false;
        self.outbox.clear();
        self.awaiting.clear();
        self.next_turns = 0;

        let mut event = Event::failure(
            "omni",
            format!("session persistence failed while {action}: {error}"),
        )
        .from(&provider)
        .about(&model);
        event.session = self.session_id.clone();
        event.turn = self.turn;
        self.pending.push(event);
        self.settle(|state| {
            state.status = STOPPED.into();
            state.provider.clear();
            state.model.clear();
            state.effort.clear();
        });
    }

    /// Hand on everything recorded while reacting to the last thing that
    /// happened.
    ///
    /// Deliberately after the reaction, not during it. An `END` is recorded
    /// before the conductor has finished closing the turn, so publishing from
    /// inside `record` would tell every listener the turn had ended and, in the
    /// same breath, that the session was still busy. Listeners see the settled
    /// state or they see nothing.
    fn publish(&mut self) {
        for event in std::mem::take(&mut self.pending) {
            (self.sink)(event);
        }
    }

    /// One write of the shared snapshot, so readers never see half a change.
    fn settle(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        change(&mut state);
        state.in_turn = self.in_turn;
        state.queued = self.outbox.len() + self.awaiting.len();
    }

    fn refresh(&self) {
        let (provider, model, effort) = match &self.current {
            Some((provider, model, effort, _)) => (provider.clone(), model.clone(), effort.clone()),
            None => (String::new(), String::new(), String::new()),
        };
        let running = self
            .runner
            .as_ref()
            .map(|r| r.name().to_string())
            .unwrap_or_default();
        self.settle(|state| {
            state.ask = self.ask.clone();
            state.providers = self.providers.clone();
            state.cwd = self.config.cwd.clone();
            state.provider = if running.is_empty() {
                String::new()
            } else {
                provider
            };
            state.model = model;
            state.effort = effort;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    struct GeneratedRunner {
        generation: u64,
        delivery: Delivery,
        stopped: Arc<AtomicBool>,
    }

    impl Runner for GeneratedRunner {
        fn name(&self) -> &str {
            crate::providers::openai::NAME
        }

        fn native_id(&self) -> String {
            "thread-new".into()
        }

        fn generation(&self) -> Option<u64> {
            Some(self.generation)
        }

        fn start(&mut self, _native_id: &str, _history: &[Event]) -> Result<(), String> {
            Ok(())
        }

        fn send(&mut self, _text: &str) -> Result<Delivery, String> {
            Ok(self.delivery)
        }

        fn stop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
        }

        fn alive(&self) -> bool {
            !self.stopped.load(Ordering::SeqCst)
        }
    }

    struct BlockingRunner {
        name: String,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        stopped: Arc<AtomicBool>,
    }

    impl Runner for BlockingRunner {
        fn name(&self) -> &str {
            &self.name
        }

        fn native_id(&self) -> String {
            "slow-native".into()
        }

        fn start(&mut self, _native_id: &str, _history: &[Event]) -> Result<(), String> {
            Ok(())
        }

        fn send(&mut self, _text: &str) -> Result<Delivery, String> {
            let _ = self.entered.send(());
            self.release
                .recv()
                .map_err(|_| "the test release disappeared".to_string())?;
            Ok(Delivery::Immediate)
        }

        fn stop(&mut self) {
            self.stopped.store(true, Ordering::SeqCst);
        }

        fn alive(&self) -> bool {
            !self.stopped.load(Ordering::SeqCst)
        }
    }

    fn delivery_chat(
        name: &str,
        delivery: Delivery,
    ) -> (Chat, Handle, Arc<Mutex<Vec<Event>>>, Arc<AtomicBool>) {
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let heard = seen.clone();
        let (mut chat, handle) = Chat::open(
            name,
            vec![crate::providers::openai::NAME.into()],
            Arc::new(move |event| heard.lock().unwrap().push(event)),
        );
        let stopped = Arc::new(AtomicBool::new(false));
        chat.runner = Some(Box::new(GeneratedRunner {
            generation: 1,
            delivery,
            stopped: stopped.clone(),
        }));
        chat.current = Some((
            crate::providers::openai::NAME.into(),
            "model".into(),
            String::new(),
            Config::default(),
        ));
        (chat, handle, seen, stopped)
    }

    fn running_chat(name: &str) -> (Chat, Handle, Arc<Mutex<Vec<Event>>>, Arc<AtomicBool>) {
        delivery_chat(name, Delivery::Immediate)
    }

    fn invalid_child(name: &str) -> std::path::PathBuf {
        let blocker = crate::shared::paths::home().join(name);
        std::fs::write(&blocker, "a regular file").unwrap();
        blocker.join("cannot-exist")
    }

    fn acknowledgement(text: &str, opening: &str) -> Event {
        crate::providers::base::confirmed(text, opening)
            .from(crate::providers::openai::NAME)
            .about("model")
    }

    #[test]
    fn an_echoed_send_is_durable_only_after_its_matching_acknowledgement() {
        let _home = crate::testing::scratch_home("echo-confirmed-send");
        let (mut chat, handle, _, _) = delivery_chat("s", Delivery::Echoed);
        chat.current = None; // isolate delivery from dial resolution

        chat.dispatch("same".into());
        chat.dispatch("same".into());
        assert_eq!(handle.snapshot().status, BUSY);
        assert_eq!(handle.snapshot().queued, 2);
        assert!(chat.store.events(0).is_empty(), "pipe writes are not proof");

        chat.absorb(acknowledgement("other", event_type::START));
        assert_eq!(chat.awaiting.len(), 2, "an unrelated replay was ignored");

        chat.absorb(acknowledgement("same", event_type::START));
        chat.absorb(acknowledgement("same", event_type::INJECTED));
        chat.absorb(acknowledgement("same", event_type::INJECTED));
        let durable = chat.store.events(0);
        assert_eq!(
            durable
                .iter()
                .filter(|event| event.is(event_type::START))
                .count(),
            1
        );
        assert_eq!(
            durable
                .iter()
                .filter(|event| event.is(event_type::INJECTED))
                .count(),
            1,
            "the third duplicate had no matching pipe write"
        );
        assert_eq!(durable[1].extra["landed"], "same_turn");
        assert!(durable.iter().all(|event| !event.is(CONFIRMED)));
        let ids: BTreeSet<&str> = durable
            .iter()
            .filter(|event| event.is(event_type::START) || event.is(event_type::INJECTED))
            .filter_map(|event| event.extra[MESSAGE_ID].as_str())
            .collect();
        assert_eq!(ids.len(), 2, "duplicate text kept distinct correlations");
        assert!(Meta::open("s").pending().is_empty());
        assert_eq!(handle.snapshot().queued, 0);
    }

    #[test]
    fn an_echo_waiting_after_end_keeps_the_chat_busy_until_its_native_turn() {
        let _home = crate::testing::scratch_home("echo-after-end");
        let (mut chat, handle, _, _) = delivery_chat("s", Delivery::Echoed);
        chat.current = None;
        chat.dispatch("one".into());
        chat.absorb(acknowledgement("one", event_type::START));
        chat.dispatch("two".into());

        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        assert_eq!(handle.snapshot().status, BUSY);
        assert!(handle.snapshot().in_turn);

        chat.absorb(acknowledgement("two", event_type::START));
        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        assert_eq!(handle.snapshot().status, WAITING);
        assert!(!handle.snapshot().in_turn);
        let starts: Vec<i64> = chat
            .store
            .events(0)
            .into_iter()
            .filter(|event| event.is(event_type::START))
            .map(|event| event.turn)
            .collect();
        assert_eq!(starts, vec![0, 1]);
    }

    #[test]
    fn a_queued_native_turn_has_no_false_idle_between_ends() {
        let _home = crate::testing::scratch_home("queued-native-turn");
        let (mut chat, handle, _, _) = delivery_chat("s", Delivery::NextTurn);
        chat.current = None;
        chat.dispatch("one".into());
        chat.dispatch("two".into());
        chat.dispatch("three".into());

        let injected = chat
            .store
            .events(0)
            .into_iter()
            .find(|event| event.is(event_type::INJECTED))
            .unwrap();
        assert_eq!(injected.extra["landed"], "next_turn");
        assert_eq!(chat.next_turns, 2);
        let ids: BTreeSet<String> = chat
            .store
            .events(0)
            .into_iter()
            .filter(|event| event.is(event_type::START) || event.is(event_type::INJECTED))
            .filter_map(|event| event.extra[MESSAGE_ID].as_str().map(str::to_string))
            .collect();
        assert_eq!(ids.len(), 3);
        assert!(Meta::open("s").pending().is_empty());

        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        assert_eq!(handle.snapshot().status, BUSY);
        assert!(handle.snapshot().in_turn);
        assert_eq!(chat.next_turns, 1);

        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        assert_eq!(handle.snapshot().status, BUSY);
        assert!(handle.snapshot().in_turn);
        assert_eq!(chat.next_turns, 0);

        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        assert_eq!(handle.snapshot().status, WAITING);
        assert!(!handle.snapshot().in_turn);
    }

    #[test]
    fn pending_fifo_reopens_and_reconciles_the_append_cleanup_crash_window() {
        let _home = crate::testing::scratch_home("chat-pending-reconcile");
        let (mut original, _) = Chat::open("s", Vec::new(), Arc::new(|_| {}));
        original.enqueue("same".into()).unwrap();
        original.enqueue("same".into()).unwrap();
        original.enqueue("last".into()).unwrap();
        let accepted = original.meta.pending();

        // Simulate process death after the opening event fsync but before the
        // transactional outbox cleanup. Only the correlated first duplicate
        // has actually landed.
        let mut opening = Event::new(event_type::START)
            .saying("same")
            .with(MESSAGE_ID, accepted[0].id.clone());
        opening.turn = 0;
        original.record(opening).unwrap();
        drop(original);

        let (restored, handle) = Chat::open("s", Vec::new(), Arc::new(|_| {}));
        let queued: Vec<(String, String)> = restored
            .outbox
            .iter()
            .map(|message| (message.id.clone(), message.text.clone()))
            .collect();
        assert_eq!(
            queued,
            vec![
                (accepted[1].id.clone(), "same".into()),
                (accepted[2].id.clone(), "last".into()),
            ]
        );
        assert_eq!(restored.meta.pending(), accepted[1..]);
        assert_eq!(handle.snapshot().queued, 2);
    }

    #[test]
    fn a_cold_reopen_retries_an_acknowledged_pending_message() {
        let _home = crate::testing::scratch_home("chat-pending-retry");
        let mut meta = Meta::open("s");
        let pending = meta.enqueue("survive the crash").unwrap();
        crate::providers::test::forget_all();
        let providers = crate::providers::test::install(&["test".into()], &[]);
        let (chat, handle) = Chat::open("s", providers, Arc::new(|_| {}));
        let thread = std::thread::spawn(move || chat.run());

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline && handle.snapshot().queued != 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(handle.snapshot().queued, 0);
        assert!(Meta::open("s").pending().is_empty());
        let opening = Store::open("s")
            .events(0)
            .into_iter()
            .find(|event| event.is(event_type::START))
            .unwrap();
        assert_eq!(opening.text, "survive the crash");
        assert_eq!(opening.extra[MESSAGE_ID], pending.id);

        assert!(handle.post(Wake::Stop));
        thread.join().unwrap();
    }

    #[test]
    fn durable_send_ack_is_not_coupled_to_provider_completion() {
        let _home = crate::testing::scratch_home("chat-local-send-ack");
        crate::providers::test::forget_all();
        let providers = crate::providers::test::install(&["slow".into()], &[]);
        let (mut chat, handle) = Chat::open("s", providers, Arc::new(|_| {}));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        chat.runner = Some(Box::new(BlockingRunner {
            name: "slow".into(),
            entered: entered_tx,
            release: release_rx,
            stopped: Arc::new(AtomicBool::new(false)),
        }));
        // With no signature, the injected runner is not considered stale.
        chat.current = None;

        let (ack, accepted) = mpsc::channel();
        assert!(handle.post(Wake::SendDurable("remember me".into(), ack)));
        let thread = std::thread::spawn(move || chat.run());
        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("provider send began and is now blocked");
        assert_eq!(
            accepted.recv_timeout(Duration::from_millis(250)).unwrap(),
            Ok(()),
            "the local journal was acknowledged while provider work remained blocked"
        );
        assert_eq!(Meta::open("s").pending().len(), 1);

        release_tx.send(()).unwrap();
        assert!(handle.post(Wake::Stop));
        thread.join().unwrap();
    }

    #[test]
    fn stopping_a_durable_turn_closes_it_once_and_preserves_pending_echoes() {
        let _home = crate::testing::scratch_home("chat-interrupted-stop");
        let (mut chat, handle, _, _) = delivery_chat("s", Delivery::Immediate);
        chat.current = None;
        chat.dispatch("landed".into());
        assert!(chat.in_turn && chat.turn_recorded);

        chat.shutdown();
        let events = chat.store.events(0);
        let ends: Vec<&Event> = events
            .iter()
            .filter(|event| event.is(event_type::END))
            .collect();
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].extra["interrupted"], true);
        assert_eq!(ends[0].extra["stopped"], true);
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(!handle.snapshot().in_turn);

        let (mut echoed, echoed_handle, _, _) = delivery_chat("echo", Delivery::Echoed);
        echoed.current = None;
        echoed.dispatch("retry after reopen".into());
        assert_eq!(Meta::open("echo").pending().len(), 1);
        echoed.shutdown();
        assert_eq!(Meta::open("echo").pending().len(), 1);
        assert_eq!(echoed_handle.snapshot().queued, 1);
        assert!(!echoed_handle.snapshot().in_turn);
    }

    #[test]
    fn a_real_end_during_stop_prevents_a_duplicate_synthetic_end() {
        let _home = crate::testing::scratch_home("chat-real-stop-end");
        let (mut chat, handle, _, _) = delivery_chat("s", Delivery::Immediate);
        chat.current = None;
        chat.dispatch("landed".into());
        chat.post
            .send(Wake::Heard(Box::new(
                Event::new(event_type::END).from(crate::providers::openai::NAME),
            )))
            .unwrap();

        chat.shutdown();
        let ends: Vec<Event> = chat
            .store
            .events(0)
            .into_iter()
            .filter(|event| event.is(event_type::END))
            .collect();
        assert_eq!(ends.len(), 1);
        assert!(!ends[0].extra.contains_key("interrupted"));
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(!handle.snapshot().in_turn);
    }

    #[test]
    fn corrupt_existing_metadata_and_event_logs_stop_before_launch() {
        let _home = crate::testing::scratch_home("chat-corrupt-startup");
        std::fs::create_dir_all(crate::shared::paths::sessions()).unwrap();
        let meta_path = crate::shared::paths::meta_file("bad-meta");
        std::fs::write(&meta_path, "{valuable but malformed").unwrap();
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let heard = seen.clone();
        let (chat, handle) = Chat::open(
            "bad-meta",
            vec!["must-not-launch".into()],
            Arc::new(move |event| heard.lock().unwrap().push(event)),
        );
        std::thread::spawn(move || chat.run()).join().unwrap();
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(seen.lock().unwrap()[0].error.contains("metadata"));
        assert_eq!(
            std::fs::read_to_string(&meta_path).unwrap(),
            "{valuable but malformed"
        );

        let log_path = crate::shared::paths::session_file("bad-log");
        std::fs::write(&log_path, "{\"complete\":true}\nnot-json\n").unwrap();
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let heard = seen.clone();
        let (chat, handle) = Chat::open(
            "bad-log",
            vec!["must-not-launch".into()],
            Arc::new(move |event| heard.lock().unwrap().push(event)),
        );
        std::thread::spawn(move || chat.run()).join().unwrap();
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(seen.lock().unwrap()[0].error.contains("session log"));
        assert_eq!(
            std::fs::read_to_string(log_path).unwrap(),
            "{\"complete\":true}\nnot-json\n"
        );
    }

    #[test]
    fn every_session_setting_and_the_first_client_cwd_survive_a_cold_reopen() {
        let _home = crate::testing::scratch_home("chat-settings-reopen");
        let first_dir = crate::shared::paths::home().join("client-one");
        let later_dir = crate::shared::paths::home().join("client-two");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&later_dir).unwrap();
        let first_dir = std::fs::canonicalize(first_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let later_dir = std::fs::canonicalize(later_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let (mut original, original_handle) = Chat::open_at(
            "s",
            vec!["alpha".into()],
            Some(&first_dir),
            Arc::new(|_| {}),
        );
        assert_eq!(original_handle.snapshot().cwd, first_dir);
        original.meta.bind("alpha", "native-alpha").unwrap();
        original
            .apply(Change::Providers(vec!["beta".into()]))
            .unwrap();
        original.apply(Change::Model(Ask::intelligence(9))).unwrap();
        original
            .apply(Change::SystemPrompt("replace everything".into()))
            .unwrap();
        original
            .apply(Change::AppendSystemPrompt("and append this".into()))
            .unwrap();
        original.apply(Change::Subagents(true)).unwrap();
        original.apply(Change::Mcp(true)).unwrap();
        original.apply(Change::Autoremove(false)).unwrap();
        drop(original);

        let (mut restored, restored_handle) = Chat::open_at(
            "s",
            vec!["fallback".into()],
            Some(&later_dir),
            Arc::new(|_| {}),
        );
        let state = restored_handle.snapshot();
        assert_eq!(state.providers, ["beta"]);
        assert_eq!(state.ask, Ask::intelligence(9));
        assert_eq!(state.cwd, first_dir, "the later opener cannot move it");
        assert_eq!(restored.config.system_prompt, "replace everything");
        assert_eq!(restored.config.append_system_prompt, "and append this");
        assert!(!restored.config.disable_subagents);
        assert!(!restored.config.disable_mcp);
        assert!(!restored.autoremove);
        assert_eq!(restored.meta.native("alpha").0, "native-alpha");

        restored.apply(Change::Cwd(later_dir.clone())).unwrap();
        assert_eq!(restored_handle.snapshot().cwd, later_dir);
        drop(restored);

        let (_, moved_handle) = Chat::open_at("s", vec![], Some(&first_dir), Arc::new(|_| {}));
        assert_eq!(
            moved_handle.snapshot().cwd,
            later_dir,
            "an explicit cwd change becomes the new pin"
        );
    }

    #[test]
    fn a_setting_metadata_failure_does_not_apply_the_requested_value() {
        let _home = crate::testing::scratch_home("chat-setting-failure");
        let (mut chat, handle, seen, stopped) = running_chat("s");
        let original = chat.providers.clone();
        chat.meta.path = invalid_child("settings-meta-is-blocked");

        assert!(
            chat.apply(Change::Providers(vec!["not-durable".into()]))
                .is_err()
        );
        chat.publish();

        assert_eq!(chat.providers, original);
        assert_eq!(chat.meta.setting(ACTIVE_PROVIDERS), Some(&json!(original)));
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(stopped.load(Ordering::SeqCst));
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "omni");
        assert_eq!(events[0].seq, -1);
    }

    #[test]
    fn a_late_old_server_exit_does_not_stop_its_replacement() {
        let _home = crate::testing::scratch_home("old-codex-generation");
        let (mut chat, _) = Chat::open(
            "s",
            vec![crate::providers::openai::NAME.into()],
            Arc::new(|_| {}),
        );
        let stopped = Arc::new(AtomicBool::new(false));
        chat.runner = Some(Box::new(GeneratedRunner {
            generation: 2,
            delivery: Delivery::Immediate,
            stopped: stopped.clone(),
        }));
        chat.current = Some((
            crate::providers::openai::NAME.into(),
            "model".into(),
            String::new(),
            Config::default(),
        ));
        chat.in_turn = true;

        chat.absorb(crate::providers::openai::server::exit_event(1, 9));

        assert!(chat.runner.is_some(), "the replacement remains attached");
        assert!(chat.in_turn, "its turn remains open");
        assert!(!stopped.load(Ordering::SeqCst));
        let recorded = chat.store.events(0).pop().unwrap();
        assert!(recorded.error.contains("exited unexpectedly"));
        assert!(
            !recorded.extra.contains_key(GENERATION),
            "internal provenance is not persisted"
        );
    }

    #[test]
    fn a_late_same_provider_end_keeps_its_origin_turn_and_cannot_close_replacement() {
        let _home = crate::testing::scratch_home("old-runner-epoch");
        let (mut chat, _, _, stopped) = running_chat("s");
        chat.runner_epoch = Some(2);
        chat.epoch_turns.insert(1, 3);
        chat.epoch_turns.insert(2, 4);
        chat.turn = 4;
        chat.in_turn = true;
        chat.turn_recorded = true;

        chat.absorb(
            Event::new(event_type::END)
                .from(crate::providers::openai::NAME)
                .with(RUNNER_EPOCH, 1),
        );

        assert!(chat.runner.is_some(), "the replacement remains attached");
        assert!(chat.in_turn, "the replacement turn remains open");
        assert!(!stopped.load(Ordering::SeqCst));
        let recorded = chat.store.events(0).pop().unwrap();
        assert_eq!(recorded.turn, 3, "late output keeps its producer's turn");
        assert!(!recorded.extra.contains_key(RUNNER_EPOCH));
    }

    #[test]
    fn a_foreign_history_straggler_holds_the_watermark_across_later_native_turns() {
        let _home = crate::testing::scratch_home("foreign-history-hole");
        let (mut chat, _, _, _) = running_chat("s");
        chat.current = None; // keep this focused on provenance, not dial lookup
        chat.runner_epoch = Some(2);
        chat.epoch_turns.insert(1, 3);
        chat.epoch_turns.insert(2, 4);
        chat.turn = 4;

        chat.absorb(
            Event::new(event_type::TEXT)
                .from(crate::providers::google::NAME)
                .saying("late but preserved")
                .with(RUNNER_EPOCH, 1),
        );
        chat.absorb(
            Event::new(event_type::TEXT)
                .from(crate::providers::openai::NAME)
                .saying("late from an older native runner")
                .with(RUNNER_EPOCH, 1),
        );
        chat.absorb(
            Event::new(event_type::TEXT)
                .from(crate::providers::openai::NAME)
                .saying("already native")
                .with(RUNNER_EPOCH, 2),
        );
        chat.finish_turn();

        assert_eq!(chat.meta.native(crate::providers::openai::NAME).1, -1);

        chat.absorb(
            Event::new(event_type::TEXT)
                .from(crate::providers::openai::NAME)
                .saying("a later native turn")
                .with(RUNNER_EPOCH, 2),
        );
        chat.finish_turn();

        assert_eq!(
            chat.meta.native(crate::providers::openai::NAME).1,
            -1,
            "a later turn cannot leap the native watermark over the hole"
        );
        let missed = chat.history_for(crate::providers::openai::NAME, 0, true);
        assert_eq!(missed.len(), 2);
        assert_eq!(missed[0].text, "late but preserved");
        assert_eq!(missed[0].turn, 3);
        assert_eq!(missed[1].text, "late from an older native runner");
        assert_eq!(missed[1].extra["late"], true);
    }

    #[test]
    fn a_retryable_provider_error_does_not_tear_down_the_open_turn() {
        let _home = crate::testing::scratch_home("retryable-provider-error");
        let (mut chat, _, _, stopped) = running_chat("s");
        chat.in_turn = true;
        chat.turn_recorded = true;

        chat.absorb(
            Event::failure(CRASH, "temporary stream failure")
                .from(crate::providers::openai::NAME)
                .with("willRetry", true),
        );

        assert!(chat.runner.is_some(), "the retrying runner stays attached");
        assert!(chat.in_turn, "the provider still owns the open turn");
        assert!(!stopped.load(Ordering::SeqCst));
        let recorded = chat.store.events(0).pop().unwrap();
        assert_eq!(recorded.extra["willRetry"], true);
    }

    #[test]
    fn an_append_failure_stops_every_runner_without_publishing_the_lost_event() {
        let _home = crate::testing::scratch_home("chat-append-failure");
        let (mut chat, handle, seen, active_stopped) = running_chat("s");
        let parked_stopped = Arc::new(AtomicBool::new(false));
        chat.parked.insert(
            "parked".into(),
            Parked {
                runner: Box::new(GeneratedRunner {
                    generation: 1,
                    delivery: Delivery::Immediate,
                    stopped: parked_stopped.clone(),
                }),
                epoch: None,
                signature: chat.current.clone().unwrap(),
                synced: -1,
            },
        );
        chat.in_turn = true;
        chat.meta
            .bind(crate::providers::openai::NAME, "native-before")
            .unwrap();

        let persisted = chat.record(Event::config("persisted")).unwrap();
        chat.store.path = invalid_child("store-is-blocked");
        chat.absorb(Event::new(event_type::END).from(crate::providers::openai::NAME));
        chat.publish();

        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "persisted");
        assert_eq!(events[0].seq, persisted.seq);
        assert_eq!(events[1].kind, "omni");
        assert_eq!(events[1].seq, -1, "the terminal notice is not durable");
        assert!(events[1].error.starts_with("session persistence failed"));
        drop(events);

        assert_eq!(
            chat.store.seq(),
            persisted.seq,
            "the failed append vanished"
        );
        assert_eq!(
            chat.meta.native(crate::providers::openai::NAME),
            ("native-before".into(), -1),
            "an END that was not persisted cannot advance the watermark"
        );
        let durable = Store::open("s").events(0);
        assert_eq!(durable.len(), 1);
        assert_eq!(durable[0].text, "persisted");
        assert_eq!(handle.snapshot().seq, persisted.seq);
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(active_stopped.load(Ordering::SeqCst));
        assert!(parked_stopped.load(Ordering::SeqCst));
        assert!(chat.runner.is_none());
        assert!(chat.parked.is_empty());
    }

    #[test]
    fn a_start_append_failure_does_not_create_a_turn() {
        let _home = crate::testing::scratch_home("chat-start-append-failure");
        let (mut chat, handle, seen, active_stopped) = running_chat("s");
        // No prior signature means the already-running runner is not stale;
        // this test is about the START write, not dial resolution.
        chat.current = None;
        chat.store.path = invalid_child("start-store-is-blocked");
        chat.enqueue("not durable".into()).unwrap();

        chat.flush();
        chat.publish();

        assert_eq!(chat.turn, -1, "the failed START did not create turn zero");
        assert_eq!(chat.store.seq(), -1);
        assert_eq!(handle.snapshot().seq, -1);
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(active_stopped.load(Ordering::SeqCst));
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "omni");
        assert_eq!(
            events[0].turn, -1,
            "the terminal notice stays on the last durable turn"
        );
    }

    #[test]
    fn a_watermark_write_failure_rolls_back_and_ends_the_session() {
        let _home = crate::testing::scratch_home("chat-watermark-failure");
        let (mut chat, handle, seen, active_stopped) = running_chat("s");
        chat.meta
            .bind(crate::providers::openai::NAME, "native-before")
            .unwrap();
        let persisted = chat
            .record(Event::new(event_type::TEXT).saying("durable"))
            .unwrap();
        chat.publish();
        chat.meta.path = invalid_child("meta-is-blocked");

        chat.finish_turn();
        chat.publish();

        assert_eq!(
            chat.meta.native(crate::providers::openai::NAME),
            ("native-before".into(), -1),
            "the failed watermark did not exist in memory"
        );
        assert_eq!(handle.snapshot().seq, persisted.seq);
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(active_stopped.load(Ordering::SeqCst));
        let events = seen.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].text, "durable");
        assert_eq!(events[1].kind, "omni");
        assert_eq!(events[1].seq, -1);
        assert_eq!(Store::open("s").events(0).len(), 1);
    }
}
