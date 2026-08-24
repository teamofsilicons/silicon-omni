//! Driving a real conductor from a test, the way a client drives one.
//!
//! Everything here goes through the same door a daemon uses: post to a
//! [`Handle`], read events off the sink. Nothing reaches inside the chat.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use omni_core::chat::{Change, Chat, Handle, Snapshot, Wake};
use omni_core::events::{Event, kind};
use omni_core::providers::test as double;
use omni_core::testing::{Home, scratch_home};

pub struct Session {
    pub handle: Handle,
    pub events: Receiver<Event>,
    pub seen: Arc<Mutex<Vec<Event>>>,
    pub thread: Option<std::thread::JoinHandle<()>>,
    pub _home: Home,
}

/// A chat over the named test providers, already running.
pub fn start(name: &str, providers: &[&str]) -> Session {
    let home = scratch_home(name);
    double::forget_all();
    let installed = double::install(
        &providers.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        &[],
    );
    open(home, name, installed)
}

/// A chat over two providers with a dial you control: `beta` strong, `alpha` cheap.
pub fn two(name: &str) -> Session {
    let home = scratch_home(name);
    double::forget_all();
    let installed = double::install(
        &["beta".to_string(), "alpha".to_string()],
        &[
            double::rung("beta", "beta-big", "high"),
            double::rung("alpha", "alpha-small", "low"),
        ],
    );
    open(home, name, installed)
}

fn open(home: Home, session_id: &str, providers: Vec<String>) -> Session {
    let (tx, events) = channel();
    let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
    let kept = seen.clone();
    let (chat, handle) = Chat::open(
        session_id,
        providers,
        Arc::new(move |event: Event| {
            kept.lock().unwrap().push(event.clone());
            let _ = tx.send(event);
        }),
    );
    let thread = std::thread::spawn(move || chat.run());
    let session = Session {
        handle,
        events,
        seen,
        thread: Some(thread),
        _home: home,
    };
    session.settle();
    session
}

impl Session {
    pub fn send(&self, text: &str) {
        self.handle.send(text);
    }

    pub fn set(&self, change: Change) {
        self.handle.post(Wake::Set(change));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.handle.snapshot()
    }

    /// Wait until the message has actually reached a provider and a turn is
    /// open. What a test needs before it can make something happen mid-turn.
    pub fn settle_started(&self) -> bool {
        self.until(|snapshot| snapshot.in_turn)
    }

    fn until(&self, want: impl Fn(&Snapshot) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if want(&self.snapshot()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Stop, then open the same session again on the same home — what a client
    /// reconnecting to a session the daemon has let go cold does.
    pub fn reopen(&mut self, providers: &[&str]) {
        self.stop();
        let installed = double::install(
            &providers.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            &[],
        );
        let (tx, events) = channel();
        let seen: Arc<Mutex<Vec<Event>>> = Arc::default();
        let kept = seen.clone();
        let (chat, handle) = Chat::open(
            &self.handle.session_id.clone(),
            installed,
            Arc::new(move |event: Event| {
                kept.lock().unwrap().push(event.clone());
                let _ = tx.send(event);
            }),
        );
        self.thread = Some(std::thread::spawn(move || chat.run()));
        self.handle = handle;
        self.events = events;
        self.seen = seen;
        self.settle();
    }

    /// Wait until the chat has nothing left to do. Returns false on timeout.
    pub fn settle(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            // Look twice: a queue can be empty a moment before the answer it is
            // waiting on arrives.
            if self.snapshot().idle() {
                std::thread::sleep(Duration::from_millis(20));
                if self.snapshot().idle() {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    pub fn log(&self) -> Vec<Event> {
        self.seen.lock().unwrap().clone()
    }

    pub fn kinds(&self) -> Vec<String> {
        self.log().into_iter().map(|event| event.kind).collect()
    }

    pub fn said(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|event| event.is(kind::TEXT))
            .map(|event| event.text)
            .collect()
    }

    /// Every `config` event with this name, in order.
    pub fn notices(&self, what: &str) -> Vec<Event> {
        self.log()
            .into_iter()
            .filter(|event| event.is(kind::CONFIG) && event.text == what)
            .collect()
    }

    pub fn count(&self, of: &str) -> usize {
        self.kinds().iter().filter(|kind| *kind == of).count()
    }

    pub fn stop(&mut self) {
        self.handle.post(Wake::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
