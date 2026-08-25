//! Every session the daemon is holding open, and who is listening to each.
//!
//! A session lives here from the first client that opens it until it is stopped
//! or goes cold. Clients come and go underneath it — that is the whole point of
//! a daemon: the provider stays hot, so the next attach costs nothing.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omni_core::chat::{Change, Chat, Handle, Snapshot, Wake};
use omni_core::events::Event;
use omni_core::session::Store;
use omni_core::wire::Frame;

use crate::wiring::{Connection, Listener};

/// How long a session with nobody listening stays hot before it is put down.
/// Long enough that a script re-running keeps its warm provider; short enough
/// that a forgotten session is not a process that lives forever.
pub fn idle_grace() -> Duration {
    let seconds = std::env::var("OMNI_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(900);
    Duration::from_secs(seconds)
}

pub struct Live {
    pub handle: Handle,
    listeners: Arc<Mutex<Vec<Arc<Listener>>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    stopping: AtomicBool,
    /// When the last listener left, if there is none.
    alone_since: Mutex<Option<Instant>>,
}

impl Live {
    /// Attach a connection, replaying what it has not seen.
    ///
    /// The listener list is held throughout, so the conductor cannot slip an
    /// event in between the replay and the subscription — no gap, no repeat.
    fn attach(&self, conn: Arc<Connection>, from: i64) -> Result<usize, String> {
        let mut listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
        let session = &self.handle.session_id;
        if !conn.open() {
            return Err(format!(
                "connection closed while session {session:?} was being opened"
            ));
        }
        if self.stopping.load(Ordering::SeqCst)
            || self.handle.snapshot().status == omni_core::chat::STOPPED
        {
            return Err(format!(
                "session {session:?} stopped while it was being opened"
            ));
        }
        // Opening the same session twice on one connection replaces its cursor;
        // it must not subscribe the connection twice to every future event.
        listeners.retain(|listener| listener.conn.id != conn.id);
        // A negative `from` means "only what happens next" — the one thing a
        // client cannot ask for by number, because it does not know the number
        // until it has attached.
        let from = match from < 0 {
            true => self.handle.snapshot().seq + 1,
            false => from,
        };
        let conn_id = conn.id;
        let listener = Listener::new(conn, from);
        let replayed = Store::open(session).events(from);
        let snapshot = self.handle.snapshot();
        for event in &replayed {
            if !listener.replay(session, event, snapshot.clone()) {
                break;
            }
        }
        listeners.push(listener.clone());
        // `talk` closes the connection before its one forget pass. Checking
        // after insertion closes the only dangerous race: if forget already
        // ran, `open` is false and we remove ourselves; if it has not, that
        // pass will see this listener.
        if !listener.conn.open() {
            listeners.retain(|listener| listener.conn.id != conn_id);
            if listeners.is_empty() {
                *self.alone_since.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
            }
            return Err(format!(
                "connection closed while session {session:?} was being opened"
            ));
        }
        *self.alone_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(replayed.len())
    }

    pub fn detach(&self, conn_id: u64) {
        let mut listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
        listeners.retain(|listener| listener.conn.id != conn_id);
        if listeners.is_empty() {
            *self.alone_since.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
        }
    }

    pub fn listeners(&self) -> usize {
        self.listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// Has this been sitting with nobody listening for longer than the grace?
    fn cold(&self, grace: Duration) -> bool {
        let snapshot = self.handle.snapshot();
        if snapshot.status == omni_core::chat::BUSY || snapshot.in_turn {
            return false;
        }
        match *self.alone_since.lock().unwrap_or_else(|p| p.into_inner()) {
            Some(since) => since.elapsed() > grace,
            None => false,
        }
    }

    fn farewell(&self) {
        let snapshot = self.handle.snapshot();
        let frame = Frame::gone(&self.handle.session_id, snapshot);
        for listener in self
            .listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
        {
            listener.conn.frame(&frame);
        }
    }

    fn shut(&self) {
        // Serialise the decision with attach: either the listener gets in first
        // and receives the farewell, or it sees `stopping` and gets an error.
        {
            let _listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
            if self.stopping.swap(true, Ordering::SeqCst) {
                return;
            }
        }
        self.handle.post(Wake::Stop);
        let thread = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        self.farewell();
    }
}

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<BTreeMap<String, Arc<Live>>>,
    /// Per-session lifecycle exclusion. Provider discovery may be slow, so it
    /// must not happen under the global sessions mutex; this still ensures two
    /// concurrent opens cannot both start the same id.
    gates: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    closing: AtomicBool,
}

impl Registry {
    /// The session by that id, started if this is the first anyone asked.
    ///
    /// Open-time settings apply whether the session is new or already warm.
    /// That makes constructing a second client equivalent to calling the same
    /// setters on the first one: the newest request wins at the next boundary.
    /// `providers` is a *thunk*: working out which CLIs are usable means running
    /// them, and a session that is already open does not need the answer. Only
    /// the client that actually starts one pays for it.
    pub fn open(
        &self,
        session_id: &str,
        providers: impl FnOnce() -> Vec<String>,
        settings: Vec<omni_core::chat::Change>,
        opening_cwd: Option<&str>,
        conn: Arc<Connection>,
        from: i64,
    ) -> Result<(Arc<Live>, usize), String> {
        self.with_lifecycle(session_id, move || {
            self.open_locked(session_id, providers, settings, opening_cwd, conn, from)
        })
    }

    fn open_locked(
        &self,
        session_id: &str,
        providers: impl FnOnce() -> Vec<String>,
        settings: Vec<Change>,
        opening_cwd: Option<&str>,
        conn: Arc<Connection>,
        from: i64,
    ) -> Result<(Arc<Live>, usize), String> {
        if self.closing.load(Ordering::SeqCst) {
            return Err("the omni daemon is shutting down".into());
        }

        let stale = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(live) = sessions.get(session_id) {
                if live.handle.snapshot().status != omni_core::chat::STOPPED {
                    let live = live.clone();
                    drop(sessions);
                    live.handle.configure(settings)?;
                    let replayed = live.attach(conn, from)?;
                    return Ok((live, replayed));
                }
            }
            sessions.remove(session_id)
        };
        if let Some(stale) = stale {
            stale.shut();
        }

        // Authentication probes can take tens of seconds. The per-id gate is
        // held, but unrelated sessions remain fully usable.
        let providers = providers();
        if !conn.open() {
            return Err(format!(
                "connection closed while session {session_id:?} was being opened"
            ));
        }
        // Serialize the final decision with shutdown. The slow work happened
        // above; this lock is held only while the in-memory Chat is assembled
        // and inserted, so shutdown can never take an empty map and then have
        // a late opener leave a provider behind.
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if self.closing.load(Ordering::SeqCst) {
            return Err("the omni daemon is shutting down".into());
        }
        let listeners: Arc<Mutex<Vec<Arc<Listener>>>> = Arc::default();
        let heard = listeners.clone();
        let name = session_id.to_string();
        let mirror: Arc<Mutex<Option<Handle>>> = Arc::default();
        let seen = mirror.clone();
        let (chat, handle) = Chat::open_at(
            session_id,
            providers,
            opening_cwd,
            Arc::new(move |event: Event| {
                let listeners = heard.lock().unwrap_or_else(|p| p.into_inner()).clone();
                if listeners.is_empty() {
                    return;
                }
                // The handle is set the moment `Chat::open` returns, which is
                // before the conductor thread starts — so by the time any event
                // exists there is always one to read.
                let Some(snapshot) = seen
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .as_ref()
                    .map(Handle::snapshot)
                else {
                    return;
                };
                for listener in listeners {
                    listener.deliver(&name, &event, snapshot.clone());
                }
            }),
        );
        *mirror.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle.clone());
        // Posted before the conductor starts, so they are handled before the
        // launch it queues for itself: the first provider to come up is the one
        // that was actually asked for. Even an empty batch is a readiness
        // barrier for constructor-time metadata initialization.
        let configured = handle.configure_deferred(settings)?;
        // Finish the one eager launch before `open` replies. A later `send`
        // therefore acknowledges only its local journal write and can never be
        // trapped behind provider startup that the session already queued.
        let launched = handle.launch_deferred()?;
        let thread = std::thread::Builder::new()
            .name(format!("omni:{session_id}"))
            .spawn(move || chat.run())
            .expect("a session thread");
        let live = Arc::new(Live {
            handle,
            listeners,
            thread: Mutex::new(Some(thread)),
            stopping: AtomicBool::new(false),
            // Opening is not idleness. `attach` clears this on success and a
            // later detach starts the real cold-session clock.
            alone_since: Mutex::new(None),
        });
        sessions.insert(session_id.to_string(), live.clone());
        drop(sessions);
        let opened = configured
            .recv()
            .unwrap_or_else(|_| {
                Err(format!(
                    "session {session_id:?} stopped while opening durable state"
                ))
            })
            .and_then(|()| {
                launched.recv().unwrap_or_else(|_| {
                    Err(format!(
                        "session {session_id:?} stopped while starting its provider"
                    ))
                })
            });
        if let Err(error) = opened {
            self.sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(session_id);
            live.shut();
            return Err(error);
        }
        match live.attach(conn, from) {
            Ok(replayed) => Ok((live, replayed)),
            Err(error) => {
                self.sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(session_id);
                live.shut();
                Err(error)
            }
        }
    }

    fn gate(&self, session_id: &str) -> Arc<Mutex<()>> {
        self.gates
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(session_id.to_string())
            .or_default()
            .clone()
    }

    /// Run one lifecycle mutation for a session, then reclaim its gate once no
    /// waiter can still be referring to it. Identity prevents an old caller
    /// from removing a replacement; the strong count distinguishes the map +
    /// this holder from any thread that already cloned the gate to wait.
    fn with_lifecycle<R>(&self, session_id: &str, operation: impl FnOnce() -> R) -> R {
        let gate = self.gate(session_id);
        let lifecycle = gate.lock().unwrap_or_else(|p| p.into_inner());
        let result = operation();
        {
            let mut gates = self.gates.lock().unwrap_or_else(|p| p.into_inner());
            let is_current = gates
                .get(session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &gate));
            if is_current && Arc::strong_count(&gate) == 2 {
                gates.remove(session_id);
            }
        }
        drop(lifecycle);
        result
    }

    pub fn get(&self, session_id: &str) -> Option<Arc<Live>> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
    }

    /// Accept a message under the same lifecycle boundary as stop/reap. Once
    /// this returns success the conductor has journaled it locally, so a later
    /// stop may interrupt delivery but cannot erase the accepted message.
    pub fn send(&self, session_id: &str, text: &str) -> Result<(), String> {
        self.with_lifecycle(session_id, || {
            let live = self.get(session_id).ok_or_else(|| missing(session_id))?;
            live.handle.send_durable(text)
        })
    }

    pub fn set(&self, session_id: &str, change: Change) -> Result<(), String> {
        self.with_lifecycle(session_id, || {
            let live = self.get(session_id).ok_or_else(|| missing(session_id))?;
            live.handle.set(change)
        })
    }

    pub fn stop(&self, session_id: &str) -> bool {
        self.with_lifecycle(session_id, || self.stop_locked_if(session_id, |_| true))
    }

    fn stop_locked_if(&self, session_id: &str, eligible: impl FnOnce(&Live) -> bool) -> bool {
        let live = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let should_remove = sessions.get(session_id).is_some_and(|live| eligible(live));
            should_remove.then(|| sessions.remove(session_id)).flatten()
        };
        match live {
            Some(live) => {
                live.shut();
                true
            }
            None => false,
        }
    }

    pub fn snapshots(&self) -> Vec<(Snapshot, usize)> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|live| (live.handle.snapshot(), live.listeners()))
            .collect()
    }

    /// Drop this connection from every session it was listening to.
    pub fn forget(&self, conn_id: u64) {
        for live in self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
        {
            live.detach(conn_id);
        }
    }

    /// Put down sessions nobody has listened to for a while, and any that have
    /// finished on their own. Returns how many went.
    pub fn reap(&self, grace: Duration) -> usize {
        let candidates: Vec<String> = self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, live)| {
                live.handle.snapshot().status == omni_core::chat::STOPPED || live.cold(grace)
            })
            .map(|(name, _)| name.clone())
            .collect();
        let reaped = candidates
            .into_iter()
            .filter(|name| {
                self.with_lifecycle(name, || {
                    self.stop_locked_if(name, |live| {
                        live.handle.snapshot().status == omni_core::chat::STOPPED
                            || live.cold(grace)
                    })
                })
            })
            .count();
        if reaped > 0 {
            // A codex server with no threads left on it is next to go.
            omni_core::providers::openai::server::reap_idle();
        }
        reaped
    }

    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let sessions =
            std::mem::take(&mut *self.sessions.lock().unwrap_or_else(|p| p.into_inner()));
        for (_, live) in sessions {
            live.shut();
        }
        omni_core::providers::openai::server::shutdown();
    }
}

fn missing(session_id: &str) -> String {
    format!("session {session_id:?} is not open; open it first")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use omni_core::chat::Change;
    use omni_core::providers::test as double;
    use omni_core::session::Meta;
    use omni_core::testing::scratch_home;

    fn connection(id: u64) -> (Arc<Connection>, UnixStream) {
        let (daemon, peer) = UnixStream::pair().unwrap();
        (Connection::new(id, daemon), peer)
    }

    fn wait_for(want: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if want() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn reattaching_applies_settings_without_duplicate_listeners() {
        let _home = scratch_home("registry-reattach");
        double::forget_all();
        let names = double::install(&["alpha".into(), "beta".into()], &[]);
        let registry = Registry::default();
        let (conn, _peer) = connection(1);
        let (first, _) = registry
            .open("s", || names, Vec::new(), None, conn.clone(), 0)
            .unwrap();
        let (again, _) = registry
            .open(
                "s",
                || panic!("a warm session must not probe providers"),
                vec![
                    Change::Providers(vec!["beta".into()]),
                    Change::Intelligence(10),
                ],
                None,
                conn,
                -1,
            )
            .unwrap();

        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(again.listeners(), 1);
        assert!(wait_for(|| {
            let state = again.handle.snapshot();
            state.providers == ["beta"] && state.intelligence == 10
        }));
        registry.shutdown();
    }

    #[test]
    fn open_returns_only_after_its_settings_and_config_events_are_durable() {
        let _home = scratch_home("registry-durable-open-settings");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Registry::default();
        let (conn, _peer) = connection(1);

        let (live, _) = registry
            .open(
                "s",
                || names,
                vec![Change::Intelligence(8), Change::Subagents(true)],
                None,
                conn,
                0,
            )
            .unwrap();

        assert_eq!(live.handle.snapshot().intelligence, 8);
        let meta = Meta::open("s");
        assert_eq!(meta.setting("level"), Some(&serde_json::json!(8)));
        assert_eq!(meta.setting("subagents"), Some(&serde_json::json!(true)));
        let configured: Vec<String> = Store::open("s")
            .events(0)
            .into_iter()
            .filter(|event| event.is(omni_core::events::event_type::CONFIG))
            .map(|event| event.text)
            .collect();
        assert!(configured.contains(&"intelligence".to_string()));
        assert!(configured.contains(&"subagents".to_string()));
        registry.shutdown();
    }

    #[test]
    fn lifecycle_gates_are_reclaimed_after_operations_finish() {
        let _home = scratch_home("registry-gate-cleanup");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Registry::default();
        let (conn, _peer) = connection(1);
        registry
            .open("s", || names, Vec::new(), None, conn, 0)
            .unwrap();
        assert!(registry.gates.lock().unwrap().is_empty());

        registry.set("s", Change::Intelligence(7)).unwrap();
        assert!(registry.gates.lock().unwrap().is_empty());
        registry.send("s", "durable").unwrap();
        assert!(registry.gates.lock().unwrap().is_empty());

        assert!(registry.stop("s"));
        assert!(registry.gates.lock().unwrap().is_empty());
        assert!(registry.send("s", "too late").is_err());
        assert!(
            registry
                .set("s", Change::SystemPrompt("too late".into()))
                .is_err()
        );
        assert!(registry.gates.lock().unwrap().is_empty());
        registry.shutdown();
    }

    #[test]
    fn cold_reaping_never_interrupts_a_busy_unobserved_turn() {
        let _home = scratch_home("registry-busy-reap");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Registry::default();
        let (conn, _peer) = connection(1);
        let (live, _) = registry
            .open("s", || names, Vec::new(), None, conn.clone(), 0)
            .unwrap();
        double::running("test").unwrap().set_knobs(double::Knobs {
            autoreply: false,
            ..Default::default()
        });
        live.detach(conn.id);
        registry.send("s", "still working").unwrap();
        assert!(wait_for(|| live.handle.snapshot().in_turn));

        assert_eq!(registry.reap(Duration::ZERO), 0);
        assert!(registry.get("s").is_some());

        double::running("test").unwrap().reply("done");
        assert!(wait_for(|| {
            let snapshot = live.handle.snapshot();
            snapshot.status == omni_core::chat::WAITING && !snapshot.in_turn
        }));
        assert_eq!(registry.reap(Duration::ZERO), 1);
        assert!(registry.get("s").is_none());
        registry.shutdown();
    }

    #[test]
    fn reaper_revalidates_after_a_candidate_becomes_busy() {
        let _home = scratch_home("registry-reap-revalidate");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Arc::new(Registry::default());
        let (conn, _peer) = connection(1);
        let (live, _) = registry
            .open("s", || names, Vec::new(), None, conn.clone(), 0)
            .unwrap();
        double::running("test").unwrap().set_knobs(double::Knobs {
            autoreply: false,
            ..Default::default()
        });
        live.detach(conn.id);
        std::thread::sleep(Duration::from_millis(2));

        let gate = registry.gate("s");
        let lifecycle = gate.lock().unwrap();
        let reaping = registry.clone();
        let reaper = std::thread::spawn(move || reaping.reap(Duration::ZERO));
        assert!(wait_for(|| Arc::strong_count(&gate) >= 3));

        // This bypass is test-only: it models a send that landed after the
        // candidate scan but before the reaper acquired the lifecycle gate.
        assert!(live.handle.send("became busy"));
        assert!(wait_for(|| live.handle.snapshot().in_turn));
        drop(lifecycle);

        assert_eq!(reaper.join().unwrap(), 0);
        assert!(registry.get("s").is_some());
        drop(gate);
        assert!(registry.stop("s"));
        registry.shutdown();
    }

    #[test]
    fn only_a_new_session_adopts_the_opening_clients_cwd() {
        let _home = scratch_home("registry-opening-cwd");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let first_dir = omni_core::shared::paths::home().join("first-client");
        let second_dir = omni_core::shared::paths::home().join("second-client");
        let third_dir = omni_core::shared::paths::home().join("third-client");
        for path in [&first_dir, &second_dir, &third_dir] {
            std::fs::create_dir_all(path).unwrap();
        }
        let first_dir = std::fs::canonicalize(first_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let second_dir = std::fs::canonicalize(second_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let third_dir = std::fs::canonicalize(third_dir)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let registry = Registry::default();

        let (first_conn, _first_peer) = connection(1);
        let initial_names = names.clone();
        let (first, _) = registry
            .open(
                "s",
                || initial_names,
                Vec::new(),
                Some(&first_dir),
                first_conn,
                0,
            )
            .unwrap();
        assert_eq!(first.handle.snapshot().cwd, first_dir);

        let (warm_conn, _warm_peer) = connection(2);
        let (warm, _) = registry
            .open(
                "s",
                || panic!("a warm session must not probe providers"),
                Vec::new(),
                Some(&second_dir),
                warm_conn,
                -1,
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &warm));
        assert_eq!(warm.handle.snapshot().cwd, first_dir);

        assert!(registry.stop("s"));
        let (cold_conn, _cold_peer) = connection(3);
        let (cold, _) = registry
            .open(
                "s",
                || vec!["discovery-fallback".into()],
                Vec::new(),
                Some(&third_dir),
                cold_conn,
                -1,
            )
            .unwrap();
        assert_eq!(cold.handle.snapshot().cwd, first_dir);
        assert_eq!(cold.handle.snapshot().providers, names);

        let (move_conn, _move_peer) = connection(4);
        registry
            .open(
                "s",
                || panic!("an explicit warm setting does not probe providers"),
                vec![Change::Cwd(second_dir.clone())],
                Some(&third_dir),
                move_conn,
                -1,
            )
            .unwrap();
        assert!(wait_for(|| cold.handle.snapshot().cwd == second_dir));
        registry.shutdown();
    }

    #[test]
    fn slow_discovery_blocks_only_the_same_session_and_starts_it_once() {
        let _home = scratch_home("registry-gates");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Arc::new(Registry::default());
        let (warm_conn, _warm_peer) = connection(1);
        registry
            .open("warm", || names.clone(), Vec::new(), None, warm_conn, 0)
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (first_conn, _first_peer) = connection(2);
        let first_registry = registry.clone();
        let first_names = names.clone();
        let first_calls = calls.clone();
        let first = std::thread::spawn(move || {
            first_registry
                .open(
                    "cold",
                    || {
                        first_calls.fetch_add(1, Ordering::SeqCst);
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        first_names
                    },
                    Vec::new(),
                    None,
                    first_conn,
                    0,
                )
                .unwrap()
                .0
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let (found_tx, found_rx) = mpsc::channel();
        let lookup = registry.clone();
        std::thread::spawn(move || found_tx.send(lookup.get("warm").is_some()).unwrap());
        assert!(found_rx.recv_timeout(Duration::from_millis(500)).unwrap());

        let (second_conn, _second_peer) = connection(3);
        let second_registry = registry.clone();
        let second_calls = calls.clone();
        let second = std::thread::spawn(move || {
            second_registry
                .open(
                    "cold",
                    || {
                        second_calls.fetch_add(1, Ordering::SeqCst);
                        Vec::new()
                    },
                    Vec::new(),
                    None,
                    second_conn,
                    -1,
                )
                .unwrap()
                .0
        });
        release_tx.send(()).unwrap();
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(registry.gates.lock().unwrap().is_empty());
        registry.shutdown();
    }

    #[test]
    fn a_late_attach_to_a_stopped_live_is_refused() {
        let _home = scratch_home("registry-stopped-attach");
        double::forget_all();
        let names = double::install(&["test".into()], &[]);
        let registry = Registry::default();
        let (conn, _peer) = connection(1);
        let (old, _) = registry
            .open("s", || names, Vec::new(), None, conn, 0)
            .unwrap();
        assert!(registry.stop("s"));

        let (late, _late_peer) = connection(2);
        assert!(old.attach(late, 0).is_err());
        registry.shutdown();
    }

    #[test]
    fn shutdown_refuses_an_open_that_was_still_discovering_providers() {
        let _home = scratch_home("registry-shutdown-race");
        let registry = Arc::new(Registry::default());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (conn, _peer) = connection(1);
        let opening = registry.clone();
        let thread = std::thread::spawn(move || {
            opening.open(
                "late",
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    vec!["test".into()]
                },
                Vec::new(),
                None,
                conn,
                0,
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        registry.shutdown();
        release_tx.send(()).unwrap();
        let error = match thread.join().unwrap() {
            Ok(_) => panic!("a session started after shutdown"),
            Err(error) => error,
        };
        assert!(error.contains("shutting down"));
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn a_connection_lost_during_discovery_cannot_leave_a_session_behind() {
        let _home = scratch_home("registry-disconnected-open");
        let registry = Arc::new(Registry::default());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (conn, _peer) = connection(1);
        let closing = conn.clone();
        let opening = registry.clone();
        let thread = std::thread::spawn(move || {
            opening.open(
                "late",
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    vec!["test".into()]
                },
                Vec::new(),
                None,
                conn,
                0,
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        closing.close();
        release_tx.send(()).unwrap();

        let error = match thread.join().unwrap() {
            Ok(_) => panic!("a closed connection created a session"),
            Err(error) => error,
        };
        assert!(error.contains("connection closed"));
        assert!(registry.snapshots().is_empty());
        registry.shutdown();
    }
}
