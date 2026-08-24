//! One omni session, whichever provider happens to be running it.
//!
//! Everything that matters happens on a single conductor thread: provider
//! output, user messages and settings changes all arrive on one queue and are
//! handled in order. That is why events come out one at a time, in the order
//! things actually happened, and why nothing ever changes mid-turn.
//!
//! The rule the whole design hangs off: **nothing changes mid turn**.
//! Intelligence, providers, prompts and session swaps are recorded when you ask
//! for them and applied at the next turn boundary.
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

use crate::events::{AUTH, CRASH, Event, kind};
use crate::intelligence::{Rung, resolve};
use crate::providers;
use crate::providers::base::{Config, Emit, Runner};
use crate::session::{Meta, Store};

pub const IDLE: &str = "idle";
pub const WAITING: &str = "waiting";
pub const BUSY: &str = "busy";
pub const STOPPED: &str = "stopped";

const MODEL_EVENTS: &[&str] = &[
    kind::TEXT,
    kind::THINKING,
    kind::TOOL_CALL,
    kind::TOOL_RESULT,
];

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
    Level(i64),
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
    Send(String),
    Heard(Box<Event>),
    Set(Change),
    Stop,
}

/// Where a chat has got to. Readable from anywhere; written only by the
/// conductor, once per change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub session: String,
    pub status: String,
    pub level: i64,
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
        // Flip to busy before returning, so a caller polling in a loop never
        // sees a false lull between saying something and it being picked up.
        {
            let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
            if state.status == WAITING || state.status == BUSY {
                state.status = BUSY.into();
            }
        }
        self.post(Wake::Send(text.to_string()))
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.read().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// A provider left running between uses, and the settings it was left with.
struct Parked {
    runner: Box<dyn Runner>,
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
    level: i64,
    config: Config,
    sink: Emit,
    inbox: Receiver<Wake>,
    post: Sender<Wake>,
    state: Arc<RwLock<Snapshot>>,
    runner: Option<Box<dyn Runner>>,
    /// Providers this session has used, left hot rather than killed.
    parked: BTreeMap<String, Parked>,
    /// The (provider, model, effort, config) the runner was built with.
    current: Option<Signature>,
    outbox: VecDeque<String>,
    /// Recorded, not yet handed on. See [`Chat::publish`].
    pending: Vec<Event>,
    in_turn: bool,
    stopping: bool,
    autoremove: bool,
    announced: BTreeSet<String>,
}

impl Chat {
    /// Open a session. `sink` hears every event the chat records, in order.
    pub fn open(session_id: &str, providers: Vec<String>, sink: Emit) -> (Chat, Handle) {
        let store = Store::open(session_id);
        let mut meta = Meta::open(session_id);
        let level = meta
            .get("level")
            .and_then(|value| value.as_i64())
            .unwrap_or(5);
        // Claude resumes by cwd, so a session that moves directory loses its
        // provider sessions. Pin the directory to the session the first time.
        let config = match meta.get_str("cwd") {
            Some(cwd) => Config::default().at(&cwd),
            None => Config::default(),
        };
        meta.set("cwd", json!(config.cwd));
        let (post, inbox) = channel();
        let state = Arc::new(RwLock::new(Snapshot {
            session: session_id.to_string(),
            status: IDLE.into(),
            level,
            providers: providers.clone(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: config.cwd.clone(),
            seq: store.seq(),
            in_turn: false,
            queued: 0,
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
            level,
            config,
            sink,
            inbox,
            post,
            state,
            runner: None,
            parked: BTreeMap::new(),
            current: None,
            outbox: VecDeque::new(),
            pending: Vec::new(),
            in_turn: false,
            stopping: false,
            autoremove: true,
            announced: BTreeSet::new(),
        };
        (chat, handle)
    }

    /// The one thread that owns this chat's state. Returns when it stops.
    pub fn run(mut self) {
        self.settle(|state| state.status = WAITING.into());
        let _ = self.post.send(Wake::Launch);
        while let Ok(wake) = self.inbox.recv() {
            match wake {
                Wake::Stop => break,
                Wake::Launch => {
                    self.attempt("starting up");
                }
                Wake::Send(text) => self.dispatch(text),
                Wake::Heard(event) => self.absorb(*event),
                Wake::Set(change) => self.apply(change),
            }
            self.publish();
        }
        self.shutdown();
        self.publish();
    }

    /// Where a runner's events go: back onto this chat's own queue.
    fn ear(&self) -> Emit {
        let post = self.post.clone();
        Arc::new(move |event| {
            let _ = post.send(Wake::Heard(Box::new(event)));
        })
    }

    // ------------------------------------------------------------------ setup

    fn apply(&mut self, change: Change) {
        let (what, extra) = match change {
            Change::Providers(providers) => {
                self.providers = providers;
                ("providers", json!({"providers": self.providers}))
            }
            Change::Level(level) => {
                self.level = level;
                self.meta.set("level", json!(level));
                ("intelligence", json!({"level": level}))
            }
            Change::SystemPrompt(text) => {
                let chars = text.chars().count();
                self.config.system_prompt = text;
                ("system_prompt", json!({"chars": chars}))
            }
            Change::AppendSystemPrompt(text) => {
                let chars = text.chars().count();
                self.config.append_system_prompt = text;
                ("append_system_prompt", json!({"chars": chars}))
            }
            Change::Subagents(on) => {
                self.config.disable_subagents = !on;
                // Whether a provider can honour this may have changed.
                self.announced.clear();
                ("subagents", json!({"on": on}))
            }
            Change::Mcp(on) => {
                self.config.disable_mcp = !on;
                self.announced.clear();
                ("mcp", json!({"on": on}))
            }
            Change::Autoremove(on) => {
                self.autoremove = on;
                ("autoremove_unauthenticated", json!({"on": on}))
            }
            Change::Cwd(path) => {
                self.config = self.config.clone().at(&path);
                self.meta.set("cwd", json!(self.config.cwd));
                ("cwd", json!({"cwd": self.config.cwd}))
            }
        };
        let extra = extra.as_object().cloned().unwrap_or_default();
        self.record(Event::config(what).extras(extra));
        self.refresh();
    }

    // -------------------------------------------------------------- conductor

    fn absorb(&mut self, event: Event) {
        if self.repeated(&event) {
            return;
        }
        let event = self.record(event);
        if self.stopping || self.straggler(&event) {
            return; // still logged, but there is nothing left to react to
        }
        if MODEL_EVENTS.contains(&event.kind.as_str()) {
            self.settle(|state| state.status = BUSY.into());
        } else if event.is(kind::END) {
            self.in_turn = false;
            self.finish_turn();
        } else if event.is(kind::ERROR) && FATAL.contains(&event.fault.as_str()) {
            self.fatal(&event);
        }
    }

    /// Has this provider already told us it cannot do this?
    ///
    /// Forgotten when the setting changes and when the conversation moves to
    /// another provider, which are the two moments the answer can differ.
    fn repeated(&mut self, event: &Event) -> bool {
        if !event.is(kind::CONFIG) || !ONCE.contains(&event.text.as_str()) {
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
        match &self.runner {
            Some(runner) => !event.provider.is_empty() && event.provider != runner.name(),
            None => false,
        }
    }

    /// An error that ends the turn: a crash, or a login that has gone.
    fn fatal(&mut self, event: &Event) {
        if event.fault == AUTH {
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
        self.close_turn(&name, json!({"unauthenticated": name}));
        if !self.autoremove || !self.providers.contains(&name) {
            return;
        }
        self.providers.retain(|provider| provider != &name);
        // A login can come back; ask the CLI again next time.
        providers::forget(&name);
        // Whatever it had parked is no longer usable to anyone.
        if let Some(mut stale) = self.parked.remove(&name) {
            stale.runner.stop();
        }
        self.record(
            Event::config("provider_removed")
                .from(&name)
                .with("why", "unauthenticated")
                .with("left", json!(self.providers)),
        );
        if self.providers.is_empty() {
            self.blocked("every provider is unauthenticated; log one back in and send again");
            return;
        }
        if self.attempt("provider lost its login") {
            self.flush();
        }
    }

    /// Put the runner down and end the turn it was in the middle of.
    fn close_turn(&mut self, provider: &str, extra: serde_json::Value) {
        if let Some(mut runner) = self.runner.take() {
            let name = runner.name().to_string();
            self.meta.mark_synced(&name, self.store.seq());
            runner.stop();
        }
        if self.in_turn {
            self.in_turn = false;
            let extra = extra.as_object().cloned().unwrap_or_default();
            self.record(Event::new(kind::END).from(provider).extras(extra));
        }
        self.settle(|state| state.status = WAITING.into());
    }

    fn finish_turn(&mut self) {
        self.settle(|state| state.status = WAITING.into());
        if let Some(runner) = &self.runner {
            let name = runner.name().to_string();
            self.meta.mark_synced(&name, self.store.seq());
        }
        if self.stale() && !self.attempt("settings changed") {
            return;
        }
        self.flush();
    }

    /// Queue a message, make sure something can carry it, then hand it over.
    fn dispatch(&mut self, text: String) {
        self.outbox.push_back(text);
        self.settle(|state| state.status = BUSY.into());
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
        while let Some(text) = self.outbox.front().cloned() {
            let sent = match self.runner.as_mut() {
                Some(runner) => runner.send(&text),
                None => return,
            };
            if let Err(why) = sent {
                let name = self
                    .runner
                    .as_ref()
                    .map(|r| r.name().to_string())
                    .unwrap_or_default();
                self.blocked(&format!("{name} would not take the message: {why}"));
                return;
            }
            let opening = if self.in_turn {
                kind::INJECTED
            } else {
                kind::START
            };
            self.record(Event::new(opening).saying(text));
            self.outbox.pop_front();
            self.in_turn = true;
            self.settle(|state| state.status = BUSY.into());
        }
    }

    /// Bring a provider up, and survive it refusing to come up.
    fn attempt(&mut self, why: &str) -> bool {
        match self.rebuild() {
            Ok(()) => true,
            Err(err) => {
                self.blocked(&format!("could not start a provider ({why}): {err}"));
                false
            }
        }
    }

    /// Say what went wrong and go back to waiting, rather than hanging in 'busy'.
    ///
    /// Queued messages stay queued: the next send retries the whole thing.
    fn blocked(&mut self, why: &str) {
        self.record(Event::failure(CRASH, why));
        self.settle(|state| state.status = WAITING.into());
    }

    // ---------------------------------------------------------------- runners

    fn rung(&self) -> Result<Rung, String> {
        resolve(self.level, &self.providers).map_err(|err| err.to_string())
    }

    fn signature(&self, rung: &Rung) -> Signature {
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
    fn tunable(&self, rung: &Rung) -> bool {
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
                self.record(
                    Event::config("retune")
                        .from(&rung.provider)
                        .about(&rung.model)
                        .with("effort", rung.effort.clone())
                        .with("level", rung.level),
                );
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
            self.park(runner);
        }
        if previous != rung.provider {
            // A new provider gets to say what it cannot do.
            self.announced.clear();
            if !previous.is_empty() {
                self.record(
                    Event::new(kind::SWITCH_PROVIDER)
                        .from(&rung.provider)
                        .about(&rung.model)
                        .with("from", previous)
                        .with("to", rung.provider.clone())
                        .with("level", rung.level),
                );
            }
        }
        self.launch(&rung)
    }

    /// Set a provider aside still running, so coming back costs nothing.
    fn park(&mut self, mut runner: Box<dyn Runner>) {
        let name = runner.name().to_string();
        let native = runner.native_id();
        if !native.is_empty() {
            self.meta.bind(&name, &native);
        }
        let synced = self.meta.native(&name).1;
        if !runner.parkable() || self.current.is_none() {
            runner.stop();
            return;
        }
        let signature = self.current.clone().expect("checked just above");
        self.parked.insert(
            name,
            Parked {
                runner,
                signature,
                synced,
            },
        );
    }

    /// Take a parked provider back, if it can be brought up to date in place.
    ///
    /// A parked runner has heard nothing since it was set aside. Some providers
    /// can be told (codex accepts items on a live thread, agy folds text into
    /// the next message); Claude cannot, because it read its history from a
    /// file once. Whoever cannot is stopped and launched again, which is always
    /// correct — just not free.
    fn unpark(&mut self, provider: &str) -> Option<Box<dyn Runner>> {
        let mut parked = self.parked.remove(provider)?;
        if !parked.runner.alive() {
            return None;
        }
        let missed = self.store.history(parked.synced + 1);
        if parked.runner.catch_up(&missed) {
            self.meta.mark_synced(provider, self.store.seq());
            return Some(parked.runner);
        }
        parked.runner.stop();
        None
    }

    fn launch(&mut self, rung: &Rung) -> Result<(), String> {
        let (native_id, synced) = self.meta.native(&rung.provider);
        let wanted = self.signature(rung);

        // Already up and reachable? Then there is nothing to launch.
        if let Some(mut kept) = self.take_parked(&rung.provider, &wanted) {
            if kept.retune(&rung.model, &rung.effort) {
                self.adopt(kept, rung, false);
                return Ok(());
            }
            kept.stop();
        }

        let mut fresh = native_id.is_empty();
        let config = Config {
            model: rung.model.clone(),
            effort: rung.effort.clone(),
            ..self.config.clone()
        };
        let mut runner = self.bring_up(rung, &config, &native_id, synced + 1)?;
        if !native_id.is_empty() && runner.native_id() != native_id {
            // The provider no longer knows that session. Rather than carry on
            // with half a conversation, start clean and tell it everything.
            runner.stop();
            self.record(
                Event::config("reseed")
                    .from(&rung.provider)
                    .with("lost", native_id.clone())
                    .with("now", runner.native_id()),
            );
            runner = self.bring_up(rung, &config, "", 0)?;
            fresh = true;
        }
        self.adopt(runner, rung, fresh);
        Ok(())
    }

    /// A parked runner whose settings still match what is wanted.
    fn take_parked(&mut self, provider: &str, wanted: &Signature) -> Option<Box<dyn Runner>> {
        let matches = self
            .parked
            .get(provider)
            .is_some_and(|parked| parked.signature.3 == wanted.3);
        if !matches {
            if let Some(mut stale) = self.parked.remove(provider) {
                stale.runner.stop(); // its config is out of date; it cannot be reused
            }
            return None;
        }
        self.unpark(provider)
    }

    fn adopt(&mut self, runner: Box<dyn Runner>, rung: &Rung, fresh: bool) {
        let native = runner.native_id();
        let seeded = runner.seeded();
        self.runner = Some(runner);
        self.current = Some(self.signature(rung));
        self.meta.bind(&rung.provider, &native);
        if seeded {
            self.meta.mark_synced(&rung.provider, self.store.seq());
        }
        self.record(
            Event::config("launch")
                .from(&rung.provider)
                .about(&rung.model)
                .with("effort", rung.effort.clone())
                .with("level", rung.level)
                .with("native", native.clone()),
        );
        if fresh {
            self.record(
                Event::new(kind::NEW_SESSION)
                    .from(&rung.provider)
                    .about(&rung.model)
                    .with("native", native),
            );
        }
        self.refresh();
    }

    fn bring_up(
        &mut self,
        rung: &Rung,
        config: &Config,
        native_id: &str,
        since: i64,
    ) -> Result<Box<dyn Runner>, String> {
        let mut runner = providers::runner(&rung.provider, &self.session_id, config, self.ear())
            .ok_or_else(|| format!("unknown provider {:?}", rung.provider))?;
        let history = self.store.history(since);
        runner.start(native_id, &history)?;
        Ok(runner)
    }

    fn shutdown(&mut self) {
        self.stopping = true;
        if let Some(mut runner) = self.runner.take() {
            if self.in_turn {
                runner.interrupt(); // ask nicely before closing the pipe
            }
            let name = runner.name().to_string();
            let native = runner.native_id();
            if !native.is_empty() {
                self.meta.bind(&name, &native);
            }
            runner.stop();
        }
        for (_, mut parked) in std::mem::take(&mut self.parked) {
            parked.runner.stop();
        }
        self.drain();
        self.record(Event::config("stop"));
        self.settle(|state| state.status = STOPPED.into());
    }

    /// Whatever the provider said on its way out still belongs in the log.
    fn drain(&mut self) {
        loop {
            match self.inbox.try_recv() {
                Ok(Wake::Heard(event)) => {
                    self.record(*event);
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return,
            }
        }
    }

    // ----------------------------------------------------------------- record

    /// Persist, then tell everyone. The session file is the event log.
    fn record(&mut self, mut event: Event) -> Event {
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
        let event = self.store.append(event);
        let seq = event.seq;
        self.settle(|state| state.seq = seq);
        self.pending.push(event.clone());
        event
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
        state.queued = self.outbox.len();
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
            state.level = self.level;
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
