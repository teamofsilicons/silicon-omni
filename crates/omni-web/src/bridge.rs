//! Holding the daemon open on behalf of the browsers.
//!
//! A website cannot keep a Unix socket. It has a series of short HTTP requests
//! and, if it wants to watch a turn happen, one long one. The bridge turns
//! that back into what the daemon expects: a connection per grant that stays
//! attached between requests, and a fan-out so any number of tabs can read the
//! same session without any number of attachments behind it.
//!
//! One connection per grant, not one for the whole bridge, because the daemon
//! treats a connection as the identity of a listener. Two sites watching the
//! same session are two listeners, and it should be able to say so.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use silicon_omni::raw::{Client, Frame, OpenOptions};

/// Frames kept for a reader that is briefly behind. Enough to cover a browser
/// reconnecting mid-turn; past it, a reader is told to go back to history
/// rather than quietly given a stream with a hole in it.
const RING: usize = 512;
/// How long an attachment nobody asked for by name outlives its last reader.
/// A browser reconnecting an `EventSource` is back well inside this.
const GRACE: Duration = Duration::from_secs(30);

/// A frame, and where it sits in the fan-out.
#[derive(Clone)]
pub struct Numbered {
    pub cursor: u64,
    pub frame: Frame,
}

#[derive(Default)]
struct Ring {
    frames: VecDeque<Numbered>,
    next: u64,
    /// How many fell off the back before a slow reader got to them.
    dropped: u64,
    gone: bool,
}

/// One session, held open on one grant's connection, read by anyone.
pub struct Attachment {
    pub session: String,
    /// True when a caller opened it by name and expects it to stay warm.
    /// A stream that opened it on the way past does not.
    explicit: AtomicBool,
    readers: AtomicUsize,
    idle_since: Mutex<Option<std::time::Instant>>,
    ring: Mutex<Ring>,
    arrived: Condvar,
    closing: AtomicBool,
}

/// What a reader gets when it asks for the next frames.
pub enum Next {
    Frames(Vec<Numbered>),
    /// The reader fell behind the ring. Its stream has a hole in it, and the
    /// only honest thing to do is say so and let it refetch from history.
    Lagged { to: u64, missed: u64 },
    /// Nothing new before the deadline. The caller sends a keepalive.
    Quiet,
    /// The session ended, or the attachment was let go.
    Over,
}

impl Attachment {
    fn new(session: &str, explicit: bool) -> Arc<Self> {
        Arc::new(Attachment {
            session: session.to_string(),
            explicit: AtomicBool::new(explicit),
            readers: AtomicUsize::new(0),
            idle_since: Mutex::new(Some(std::time::Instant::now())),
            ring: Mutex::new(Ring::default()),
            arrived: Condvar::new(),
            closing: AtomicBool::new(false),
        })
    }

    /// The cursor a reader starting now should ask from.
    pub fn cursor(&self) -> u64 {
        self.ring.lock().unwrap_or_else(|p| p.into_inner()).next
    }

    pub fn push(&self, frame: Frame) {
        let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        let cursor = ring.next;
        ring.next += 1;
        ring.frames.push_back(Numbered { cursor, frame });
        while ring.frames.len() > RING {
            ring.frames.pop_front();
            ring.dropped += 1;
        }
        self.arrived.notify_all();
    }

    fn finish(&self) {
        let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        ring.gone = true;
        self.arrived.notify_all();
    }

    /// Wait for anything after `from`, or for `patience` to run out.
    pub fn next(&self, from: u64, patience: Duration) -> (Next, u64) {
        let mut ring = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if self.closing.load(Ordering::SeqCst) {
                return (Next::Over, from);
            }
            let oldest = ring.frames.front().map(|held| held.cursor);
            if let Some(oldest) = oldest
                && from < oldest
            {
                let missed = oldest - from;
                return (Next::Lagged { to: oldest, missed }, oldest);
            }
            let ready: Vec<Numbered> = ring
                .frames
                .iter()
                .filter(|held| held.cursor >= from)
                .cloned()
                .collect();
            if let Some(last) = ready.last() {
                let resume = last.cursor + 1;
                return (Next::Frames(ready), resume);
            }
            if ring.gone {
                return (Next::Over, from);
            }
            let (held, timeout) = self
                .arrived
                .wait_timeout(ring, patience)
                .unwrap_or_else(|p| p.into_inner());
            ring = held;
            if timeout.timed_out() {
                return (Next::Quiet, from);
            }
        }
    }

    fn joined(&self) {
        self.readers.fetch_add(1, Ordering::SeqCst);
        *self.idle_since.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn left(&self) {
        if self.readers.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.idle_since.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(std::time::Instant::now());
        }
    }

    pub fn readers(&self) -> usize {
        self.readers.load(Ordering::SeqCst)
    }

    fn abandoned(&self) -> bool {
        if self.explicit.load(Ordering::SeqCst) || self.readers() > 0 {
            return false;
        }
        self.idle_since
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|since| since.elapsed() > GRACE)
    }
}

/// Counts a reader in for as long as it is reading.
pub struct Reading(Arc<Attachment>);

impl Reading {
    pub fn attachment(&self) -> &Attachment {
        &self.0
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        self.0.left();
    }
}

/// Everything one grant is holding: its own daemon connection, and the
/// sessions it has open on it.
struct Link {
    client: Mutex<Option<Client>>,
    sessions: Mutex<HashMap<String, Arc<Attachment>>>,
}

impl Link {
    fn new() -> Self {
        Link {
            client: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The connection, made or remade. A daemon that went away and came back
    /// leaves a dead client behind; replacing it here is the only place that
    /// has to notice.
    fn client(&self) -> Result<Client, String> {
        let mut held = self.client.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(client) = held.as_ref()
            && client.is_connected()
        {
            return Ok(client.clone());
        }
        // Every attachment on the old connection died with it. Forget them so
        // the next open builds a real one rather than handing back a corpse.
        for (_, attachment) in self
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain()
        {
            attachment.closing.store(true, Ordering::SeqCst);
            attachment.finish();
        }
        let fresh = Client::connect().map_err(|error| error.to_string())?;
        *held = Some(fresh.clone());
        Ok(fresh)
    }
}

/// The whole fan-out, keyed by grant.
pub struct Bridge {
    links: Mutex<HashMap<String, Arc<Link>>>,
    streams: AtomicU64,
}

impl Default for Bridge {
    fn default() -> Self {
        Bridge {
            links: Mutex::new(HashMap::new()),
            streams: AtomicU64::new(0),
        }
    }
}

impl Bridge {
    fn link(&self, grant: &str) -> Arc<Link> {
        self.links
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(grant.to_string())
            .or_insert_with(|| Arc::new(Link::new()))
            .clone()
    }

    /// The daemon connection this grant speaks on.
    pub fn client(&self, grant: &str) -> Result<Client, String> {
        self.link(grant).client()
    }

    /// Attach `session` on this grant's connection, or hand back the one that
    /// is already attached.
    ///
    /// `explicit` marks an attachment the caller wants kept warm. An implicit
    /// one — opened because somebody started reading — is let go a little
    /// after the last reader leaves.
    pub fn open(
        &self,
        grant: &str,
        session: &str,
        providers: Option<Vec<String>>,
        settings: Vec<(String, serde_json::Value)>,
        from: i64,
        explicit: bool,
    ) -> Result<(Arc<Attachment>, silicon_omni::raw::Opened), String> {
        let link = self.link(grant);
        let client = link.client()?;

        {
            let held = link.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(attachment) = held.get(session) {
                if explicit {
                    attachment.explicit.store(true, Ordering::SeqCst);
                }
                let attachment = attachment.clone();
                drop(held);
                // Already attached, so there is nothing to replay and no new
                // listener to count. Settings still have to land, because a
                // caller that asked for a model expects it applied whether or
                // not somebody else opened the session first.
                for (what, value) in settings {
                    client
                        .set(session, &what, value)
                        .map_err(|error| error.to_string())?;
                }
                let (snapshot, listeners) =
                    client.status(session).map_err(|error| error.to_string())?;
                return Ok((
                    attachment,
                    silicon_omni::raw::Opened {
                        session: session.to_string(),
                        replayed: 0,
                        listeners,
                        snapshot,
                    },
                ));
            }
        }

        let mut options = OpenOptions::new(session).from_seq(from);
        if let Some(providers) = providers {
            options = options.providers(providers);
        }
        for (what, value) in settings {
            options = options.setting(what, value);
        }
        let attached = client.open(options).map_err(|error| error.to_string())?;
        let opened = attached.opened().clone();
        let attachment = Attachment::new(session, explicit);

        {
            let mut held = link.sessions.lock().unwrap_or_else(|p| p.into_inner());
            // Two requests can race to open the same session. The loser's
            // attachment is dropped, which detaches it, and both callers get
            // the one that won.
            if let Some(existing) = held.get(session) {
                let existing = existing.clone();
                drop(held);
                drop(attached);
                return Ok((existing, opened));
            }
            held.insert(session.to_string(), attachment.clone());
        }

        let pump = attachment.clone();
        let link_for_pump = link.clone();
        let name = session.to_string();
        let started = std::thread::Builder::new()
            .name(format!("omni-web:{session}"))
            .spawn(move || {
                let mut attached = attached;
                loop {
                    match attached.recv() {
                        Ok(frame) => {
                            let over = frame.stream == "gone";
                            pump.push(frame);
                            if over {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                pump.finish();
                // The session ended on its own. Take it out of the map so the
                // next open builds a live one instead of subscribing to a
                // stream that will never say anything again.
                let mut held = link_for_pump
                    .sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if held
                    .get(&name)
                    .is_some_and(|current| Arc::ptr_eq(current, &pump))
                {
                    held.remove(&name);
                }
                let _ = attached.detach();
            });
        if let Err(error) = started {
            link.sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(session);
            return Err(format!("cannot watch {session:?}: {error}"));
        }

        Ok((attachment, opened))
    }

    /// Start reading. The attachment stays alive for as long as the returned
    /// guard does.
    pub fn read(&self, attachment: Arc<Attachment>) -> Reading {
        attachment.joined();
        self.streams.fetch_add(1, Ordering::SeqCst);
        Reading(attachment)
    }

    pub fn finished_reading(&self) {
        self.streams.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn streaming(&self) -> u64 {
        self.streams.load(Ordering::SeqCst)
    }

    /// Let a session go. The daemon keeps the provider warm; only this
    /// bridge's listener goes away.
    pub fn detach(&self, grant: &str, session: &str) -> bool {
        let link = self.link(grant);
        let taken = link
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session);
        match taken {
            Some(attachment) => {
                attachment.closing.store(true, Ordering::SeqCst);
                attachment.finish();
                true
            }
            None => false,
        }
    }

    /// Everything this grant is holding, by name.
    pub fn attached(&self, grant: &str) -> Vec<(String, usize)> {
        self.link(grant)
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(name, attachment)| (name.clone(), attachment.readers()))
            .collect()
    }

    /// Drop a grant entirely: its sessions, and the connection under them.
    pub fn forget(&self, grant: &str) {
        let Some(link) = self
            .links
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(grant)
        else {
            return;
        };
        for (_, attachment) in link
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain()
        {
            attachment.closing.store(true, Ordering::SeqCst);
            attachment.finish();
        }
    }

    pub fn forget_all(&self) {
        let names: Vec<String> = self
            .links
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect();
        for name in names {
            self.forget(&name);
        }
    }

    /// Release attachments that were only ever opened on the way past and
    /// have had no reader for a while.
    pub fn sweep(&self) {
        let links: Vec<Arc<Link>> = self
            .links
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        for link in links {
            let mut held = link.sessions.lock().unwrap_or_else(|p| p.into_inner());
            held.retain(|_, attachment| {
                if attachment.abandoned() {
                    attachment.closing.store(true, Ordering::SeqCst);
                    attachment.finish();
                    return false;
                }
                true
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omni_core::Event;
    use omni_core::choose::Ask;
    use silicon_omni::Snapshot;

    fn snapshot() -> Snapshot {
        Snapshot {
            session: "demo".into(),
            status: "waiting".into(),
            ask: Ask::intelligence(5),
            providers: Vec::new(),
            provider: String::new(),
            model: String::new(),
            effort: String::new(),
            cwd: "/tmp".into(),
            seq: 0,
            in_turn: false,
            queued: 0,
        }
    }

    fn text(body: &str) -> Frame {
        let mut event = Event::new(omni_core::events::event_type::TEXT);
        event.text = body.into();
        Frame::event("demo", event, snapshot())
    }

    #[test]
    fn a_reader_that_arrives_late_still_gets_what_is_held() {
        let attachment = Attachment::new("demo", false);
        attachment.push(text("one"));
        attachment.push(text("two"));

        let (next, resume) = attachment.next(0, Duration::from_millis(10));
        let Next::Frames(frames) = next else {
            panic!("expected the two frames already held")
        };
        assert_eq!(frames.len(), 2);
        assert_eq!(resume, 2);
    }

    #[test]
    fn two_readers_at_different_places_each_get_their_own() {
        let attachment = Attachment::new("demo", false);
        attachment.push(text("one"));
        let (_, ahead) = attachment.next(0, Duration::from_millis(10));
        attachment.push(text("two"));

        let (caught_up, _) = attachment.next(ahead, Duration::from_millis(10));
        let Next::Frames(frames) = caught_up else {
            panic!("the second frame should be waiting")
        };
        assert_eq!(frames.len(), 1);

        let (from_scratch, _) = attachment.next(0, Duration::from_millis(10));
        let Next::Frames(frames) = from_scratch else {
            panic!("a reader starting over sees both")
        };
        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn a_reader_that_falls_off_the_ring_is_told_rather_than_shortchanged() {
        let attachment = Attachment::new("demo", false);
        for index in 0..RING + 10 {
            attachment.push(text(&index.to_string()));
        }
        let (next, resume) = attachment.next(0, Duration::from_millis(10));
        let Next::Lagged { to, missed } = next else {
            panic!("a hole should be reported, not papered over")
        };
        assert_eq!(missed, 10);
        assert_eq!(to, 10);
        assert_eq!(resume, 10);
    }

    #[test]
    fn nothing_happening_comes_back_quiet_so_a_keepalive_can_go_out() {
        let attachment = Attachment::new("demo", false);
        let (next, resume) = attachment.next(0, Duration::from_millis(20));
        assert!(matches!(next, Next::Quiet));
        assert_eq!(resume, 0, "a quiet wait does not move the cursor");
    }

    #[test]
    fn a_session_that_ends_ends_the_stream() {
        let attachment = Attachment::new("demo", false);
        attachment.push(Frame::gone("demo", snapshot()));
        attachment.finish();
        let (first, resume) = attachment.next(0, Duration::from_millis(10));
        assert!(matches!(first, Next::Frames(_)), "the gone frame goes out");
        let (then, _) = attachment.next(resume, Duration::from_millis(10));
        assert!(matches!(then, Next::Over));
    }

    #[test]
    fn an_attachment_nobody_asked_for_is_let_go_and_an_explicit_one_is_not() {
        let implicit = Attachment::new("demo", false);
        let explicit = Attachment::new("demo", true);
        // Neither has waited out the grace period yet.
        assert!(!implicit.abandoned());
        assert!(!explicit.abandoned());

        *implicit.idle_since.lock().unwrap() =
            Some(std::time::Instant::now() - GRACE - Duration::from_secs(1));
        *explicit.idle_since.lock().unwrap() =
            Some(std::time::Instant::now() - GRACE - Duration::from_secs(1));
        assert!(implicit.abandoned());
        assert!(!explicit.abandoned(), "somebody asked for this one by name");

        implicit.joined();
        assert!(!implicit.abandoned(), "it has a reader again");
    }
}
