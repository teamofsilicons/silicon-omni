//! The session file: an append-only log of [`Event`] values.
//!
//! The event stream and the conversation history are the same thing. Nothing is
//! summarised on the way in and nothing is dropped, so a session can always be
//! replayed into a provider that was not there when it happened.
//!
//! The daemon holds a session's log in memory for as long as the session is
//! live. Seeding a provider then costs a slice rather than a re-read and
//! re-parse of the whole file, which is the difference between a switch that
//! feels instant and one that does not.

use std::path::PathBuf;

use crate::events::Event;
use crate::shared::{jsonl, paths};

pub struct Store {
    pub session_id: String,
    pub path: PathBuf,
    events: Vec<Event>,
}

impl Store {
    /// Read what is already on disk. Called once per session, by the daemon.
    pub fn open(session_id: &str) -> Self {
        let path = paths::session_file(session_id);
        let events = jsonl::read(&path)
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();
        Store {
            session_id: session_id.to_string(),
            path,
            events,
        }
    }

    /// The position the next event will take.
    pub fn seq(&self) -> i64 {
        self.events.last().map_or(-1, |event| event.seq)
    }

    /// Stamp the event with its position in the session and persist it.
    pub fn append(&mut self, mut event: Event) -> Event {
        event.seq = self.seq() + 1;
        if event.session.is_empty() {
            event.session = self.session_id.clone();
        }
        let _ = jsonl::append(&self.path, &event.to_value());
        self.events.push(event.clone());
        event
    }

    pub fn events(&self, since: i64) -> Vec<Event> {
        self.events
            .iter()
            .filter(|event| event.seq >= since)
            .cloned()
            .collect()
    }

    /// Just the conversation: what a provider needs to be brought up to date.
    pub fn history(&self, since: i64) -> Vec<Event> {
        self.events
            .iter()
            .filter(|event| event.seq >= since && event.is_history())
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::kind;
    use crate::testing::scratch_home;

    #[test]
    fn seq_only_ever_goes_up_and_survives_a_reopen() {
        let _home = scratch_home("store-seq");
        let mut store = Store::open("s");
        for _ in 0..3 {
            store.append(Event::new(kind::TEXT));
        }
        assert_eq!(store.seq(), 2);
        assert_eq!(
            Store::open("s").seq(),
            2,
            "picked up where the file left off"
        );
        assert_eq!(Store::open("s").append(Event::new(kind::TEXT)).seq, 3);
    }

    #[test]
    fn history_is_the_conversation_and_nothing_else() {
        let _home = scratch_home("store-history");
        let mut store = Store::open("s");
        store.append(Event::new(kind::START).saying("hi"));
        store.append(Event::config("launch"));
        store.append(Event::new(kind::TEXT).saying("hello"));
        store.append(Event::new(kind::END));
        let said: Vec<String> = store.history(0).into_iter().map(|e| e.text).collect();
        assert_eq!(said, vec!["hi", "hello"]);
        assert_eq!(store.events(0).len(), 4, "the log itself keeps everything");
    }

    #[test]
    fn since_is_where_a_provider_left_off() {
        let _home = scratch_home("store-since");
        let mut store = Store::open("s");
        store.append(Event::new(kind::TEXT).saying("old"));
        let mark = store.seq();
        store.append(Event::new(kind::TEXT).saying("new"));
        let said: Vec<String> = store
            .history(mark + 1)
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(said, vec!["new"]);
    }
}
