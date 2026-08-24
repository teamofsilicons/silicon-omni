//! Every session the daemon is holding open, and who is listening to each.
//!
//! A session lives here from the first client that opens it until it is stopped
//! or goes cold. Clients come and go underneath it — that is the whole point of
//! a daemon: the provider stays hot, so the next attach costs nothing.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omni_core::chat::{Chat, Handle, Snapshot, Wake};
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
    /// When the last listener left, if there is none.
    alone_since: Mutex<Option<Instant>>,
}

impl Live {
    /// Attach a connection, replaying what it has not seen.
    ///
    /// The listener list is held throughout, so the conductor cannot slip an
    /// event in between the replay and the subscription — no gap, no repeat.
    pub fn attach(&self, conn: Arc<Connection>, from: i64) -> usize {
        let mut listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
        let session = &self.handle.session_id;
        // A negative `from` means "only what happens next" — the one thing a
        // client cannot ask for by number, because it does not know the number
        // until it has attached.
        let from = match from < 0 {
            true => self.handle.snapshot().seq + 1,
            false => from,
        };
        let listener = Listener::new(conn, from);
        let replayed = Store::open(session).events(from);
        let snapshot = self.handle.snapshot();
        for event in &replayed {
            listener.deliver(session, event, snapshot.clone());
        }
        listeners.push(listener);
        *self.alone_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
        replayed.len()
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
}

impl Registry {
    /// The session by that id, started if this is the first anyone asked.
    ///
    /// `providers` only applies when it is being started: an id already open is
    /// the same conversation, and reopening it must not silently change what it
    /// is allowed to run. Say so with `set` instead.
    /// `providers` is a *thunk*: working out which CLIs are usable means running
    /// them, and a session that is already open does not need the answer. Only
    /// the client that actually starts one pays for it.
    pub fn open(
        &self,
        session_id: &str,
        providers: impl FnOnce() -> Vec<String>,
        settings: Vec<omni_core::chat::Change>,
    ) -> Arc<Live> {
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(live) = sessions.get(session_id) {
            if live.handle.snapshot().status != omni_core::chat::STOPPED {
                return live.clone();
            }
            sessions.remove(session_id); // finished; a fresh one takes its place
        }
        let providers = providers();
        let listeners: Arc<Mutex<Vec<Arc<Listener>>>> = Arc::default();
        let heard = listeners.clone();
        let name = session_id.to_string();
        let mirror: Arc<Mutex<Option<Handle>>> = Arc::default();
        let seen = mirror.clone();
        let (chat, handle) = Chat::open(
            session_id,
            providers,
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
        // that was actually asked for.
        for change in settings {
            handle.post(Wake::Set(change));
        }
        let thread = std::thread::Builder::new()
            .name(format!("omni:{session_id}"))
            .spawn(move || chat.run())
            .expect("a session thread");
        let live = Arc::new(Live {
            handle,
            listeners,
            thread: Mutex::new(Some(thread)),
            alone_since: Mutex::new(Some(Instant::now())),
        });
        sessions.insert(session_id.to_string(), live.clone());
        live
    }

    pub fn get(&self, session_id: &str) -> Option<Arc<Live>> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .cloned()
    }

    pub fn stop(&self, session_id: &str) -> bool {
        let live = self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id);
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
        let cold: Vec<String> = self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, live)| {
                live.handle.snapshot().status == omni_core::chat::STOPPED || live.cold(grace)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &cold {
            self.stop(name);
        }
        if !cold.is_empty() {
            // A codex server with no threads left on it is next to go.
            omni_core::providers::openai::server::reap_idle();
        }
        cold.len()
    }

    pub fn shutdown(&self) {
        let sessions =
            std::mem::take(&mut *self.sessions.lock().unwrap_or_else(|p| p.into_inner()));
        for (_, live) in sessions {
            live.shut();
        }
        omni_core::providers::openai::server::shutdown();
    }
}
