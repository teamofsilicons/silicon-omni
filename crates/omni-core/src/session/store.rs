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

use crate::events::{Event, SCHEMA_VERSION};
use crate::shared::{jsonl, paths};

pub struct Store {
    pub session_id: String,
    pub path: PathBuf,
    events: Vec<Event>,
    load_error: Option<String>,
    appender: Option<jsonl::Appender>,
}

impl Store {
    /// Read what is already on disk. Called once per session, by the daemon.
    pub fn open(session_id: &str) -> Self {
        let path = paths::session_file(session_id);
        let report = jsonl::read_checked(&path);
        let read_error = report.error.map(|error| error.to_string());
        let mut load_error = None;
        let mut events = Vec::with_capacity(report.records.len());
        for record in report.records {
            let event: Event = match serde_json::from_value(record.value) {
                Ok(event) => event,
                Err(error) => {
                    load_error.get_or_insert_with(|| {
                        format!("invalid event on line {}: {error}", record.line)
                    });
                    break;
                }
            };
            if event.v != SCHEMA_VERSION {
                load_error.get_or_insert_with(|| {
                    format!(
                        "unsupported event schema {} on line {}; expected {SCHEMA_VERSION}",
                        event.v, record.line
                    )
                });
                break;
            }
            if event.event_type.is_empty() {
                load_error.get_or_insert_with(|| {
                    format!(
                        "invalid event on line {}: type cannot be empty",
                        record.line
                    )
                });
                break;
            }
            let expected = events.last().map_or(0, |previous: &Event| previous.seq + 1);
            if event.seq != expected {
                load_error.get_or_insert_with(|| {
                    format!(
                        "invalid event sequence {} on line {}; expected {expected}",
                        event.seq, record.line
                    )
                });
                break;
            }
            if !event.session.is_empty() && event.session != session_id {
                load_error.get_or_insert_with(|| {
                    format!(
                        "event on line {} belongs to session {:?}, not {session_id:?}",
                        record.line, event.session
                    )
                });
                break;
            }
            events.push(event);
        }
        if load_error.is_none() {
            load_error = read_error;
        }
        Store {
            session_id: session_id.to_string(),
            path,
            events,
            load_error,
            appender: None,
        }
    }

    /// Complete-line or event-schema corruption found while opening this log.
    ///
    /// Callers should refuse to start a provider for a session with this set:
    /// the in-memory events are only the known-good durable prefix.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// The position the next event will take.
    pub fn seq(&self) -> i64 {
        self.events.last().map_or(-1, |event| event.seq)
    }

    /// The newest omni turn already in the log. Pre-versioned records did not
    /// carry `turn`, so START remains a lossless fallback while old sessions
    /// are continued after an upgrade.
    pub fn turn(&self) -> i64 {
        self.events.iter().fold(-1, |turn, event| {
            if event.turn >= 0 {
                turn.max(event.turn)
            } else if event.is(crate::events::event_type::START) {
                turn + 1
            } else {
                turn
            }
        })
    }

    /// Stamp the event with its position in the session and persist it.
    pub fn append(&mut self, mut event: Event) -> std::io::Result<Event> {
        if let Some(error) = &self.load_error {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("refusing to append to a corrupt session log: {error}"),
            ));
        }
        event.seq = self.seq() + 1;
        if event.session.is_empty() {
            event.session = self.session_id.clone();
        }
        let needs_appender = self
            .appender
            .as_ref()
            .is_none_or(|appender| appender.path() != self.path);
        if needs_appender {
            if let Some(parent) = self.path.parent() {
                paths::ensure(parent)?;
            }
            // Construct the replacement completely before dropping a healthy
            // writer. Besides making path changes in tests recoverable, this
            // keeps an open failure from damaging the live Store's state.
            let appender = jsonl::Appender::open(&self.path)?;
            self.appender = Some(appender);
        }
        self.appender
            .as_mut()
            .expect("initialized immediately above")
            .append(&event.to_value())?;
        self.events.push(event.clone());
        Ok(event)
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
    use crate::events::event_type;
    use crate::testing::scratch_home;
    use std::io::Write;

    #[test]
    fn seq_only_ever_goes_up_and_survives_a_reopen() {
        let _home = scratch_home("store-seq");
        let mut store = Store::open("s");
        for _ in 0..3 {
            store.append(Event::new(event_type::TEXT)).unwrap();
        }
        assert_eq!(store.seq(), 2);
        assert_eq!(
            Store::open("s").seq(),
            2,
            "picked up where the file left off"
        );
        assert_eq!(
            Store::open("s")
                .append(Event::new(event_type::TEXT))
                .unwrap()
                .seq,
            3
        );
    }

    #[test]
    fn history_is_the_conversation_and_nothing_else() {
        let _home = scratch_home("store-history");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::START).saying("hi"))
            .unwrap();
        store.append(Event::config("launch")).unwrap();
        store
            .append(Event::new(event_type::TEXT).saying("hello"))
            .unwrap();
        store.append(Event::new(event_type::END)).unwrap();
        let said: Vec<String> = store.history(0).into_iter().map(|e| e.text).collect();
        assert_eq!(said, vec!["hi", "hello"]);
        assert_eq!(store.events(0).len(), 4, "the log itself keeps everything");
    }

    #[test]
    fn since_is_where_a_provider_left_off() {
        let _home = scratch_home("store-since");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::TEXT).saying("old"))
            .unwrap();
        let mark = store.seq();
        store
            .append(Event::new(event_type::TEXT).saying("new"))
            .unwrap();
        let said: Vec<String> = store
            .history(mark + 1)
            .into_iter()
            .map(|e| e.text)
            .collect();
        assert_eq!(said, vec!["new"]);
    }

    #[test]
    fn turn_continues_across_versioned_and_pre_versioned_logs() {
        let _home = scratch_home("store-turn");
        let mut store = Store::open("s");
        store.append(Event::new(event_type::START)).unwrap();
        store.append(Event::new(event_type::END)).unwrap();
        let mut next = Event::new(event_type::START);
        next.turn = 1;
        store.append(next).unwrap();
        assert_eq!(store.turn(), 1);
        assert_eq!(Store::open("s").turn(), 1);
    }

    #[test]
    fn a_failed_append_does_not_exist_in_memory_or_consume_a_sequence() {
        let _home = scratch_home("store-append-failure");
        let mut store = Store::open("s");
        let valid_path = store.path.clone();
        let blocker = paths::home().join("not-a-directory");
        std::fs::write(&blocker, "a regular file").unwrap();
        store.path = blocker.join("s.jsonl");

        assert!(
            store
                .append(Event::new(event_type::TEXT).saying("lost"))
                .is_err()
        );
        assert_eq!(store.seq(), -1);
        assert!(store.events(0).is_empty());

        store.path = valid_path;
        let saved = store
            .append(Event::new(event_type::TEXT).saying("kept"))
            .unwrap();
        assert_eq!(saved.seq, 0, "the failed event did not consume seq 0");
        assert_eq!(store.events(0).len(), 1);
    }

    #[test]
    fn a_failed_writer_replacement_keeps_the_live_log_and_sequence_usable() {
        let _home = scratch_home("store-writer-replacement-failure");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::TEXT).saying("before"))
            .unwrap();
        let valid_path = store.path.clone();
        let blocker = paths::home().join("not-a-directory");
        std::fs::write(&blocker, "a regular file").unwrap();
        store.path = blocker.join("s.jsonl");

        assert!(
            store
                .append(Event::new(event_type::TEXT).saying("lost"))
                .is_err()
        );
        assert_eq!(store.seq(), 0);
        assert_eq!(store.events(0).len(), 1);

        store.path = valid_path;
        let saved = store
            .append(Event::new(event_type::TEXT).saying("after"))
            .unwrap();
        assert_eq!(saved.seq, 1, "the failed event did not consume seq 1");
        let durable: Vec<String> = Store::open("s")
            .events(0)
            .into_iter()
            .map(|event| event.text)
            .collect();
        assert_eq!(durable, vec!["before", "after"]);
    }

    #[test]
    fn complete_json_corruption_is_retained_and_blocks_append() {
        let _home = scratch_home("store-malformed-json");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::TEXT).saying("safe"))
            .unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&store.path)
            .unwrap();
        file.write_all(b"{\"type\":\n").unwrap();
        drop(file);

        let mut reopened = Store::open("s");

        assert_eq!(
            reopened.events(0).len(),
            1,
            "the good prefix remains readable"
        );
        assert!(
            reopened
                .load_error()
                .is_some_and(|error| error.contains("complete line 2")),
            "the malformed complete record is not silently filtered"
        );
        assert_eq!(
            reopened
                .append(Event::new(event_type::TEXT))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn invalid_event_shape_is_retained_as_a_load_error() {
        let _home = scratch_home("store-invalid-event");
        let path = paths::session_file("s");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            concat!(
                "{\"v\":1,\"type\":\"text\",\"text\":\"safe\",\"seq\":0}\n",
                "{\"v\":1,\"type\":7,\"seq\":1}\n",
                "{\"v\":1,\"type\":\"text\",\"text\":\"later\",\"seq\":2}\n"
            ),
        )
        .unwrap();

        let store = Store::open("s");

        assert_eq!(
            store.events(0).len(),
            1,
            "loading stops at the invalid event"
        );
        assert!(
            store
                .load_error()
                .is_some_and(|error| error.contains("invalid event on line 2"))
        );
    }

    #[test]
    fn duplicate_sequences_and_foreign_session_records_are_corruption() {
        let _home = scratch_home("store-identity-corruption");
        let path = paths::session_file("s");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            concat!(
                "{\"v\":1,\"type\":\"text\",\"session\":\"s\",\"seq\":0}\n",
                "{\"v\":1,\"type\":\"text\",\"session\":\"s\",\"seq\":0}\n"
            ),
        )
        .unwrap();
        let duplicate = Store::open("s");
        assert!(
            duplicate
                .load_error()
                .is_some_and(|error| error.contains("expected 1"))
        );

        std::fs::write(
            &path,
            "{\"v\":1,\"type\":\"text\",\"session\":\"other\",\"seq\":0}\n",
        )
        .unwrap();
        let foreign = Store::open("s");
        assert!(
            foreign
                .load_error()
                .is_some_and(|error| error.contains("belongs to session"))
        );
    }

    #[test]
    fn a_valid_unterminated_event_keeps_its_sequence_before_the_next_append() {
        let _home = scratch_home("store-valid-tail-seq");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::TEXT).saying("zero"))
            .unwrap();
        let mut tail = Event::new(event_type::TEXT).saying("one");
        tail.session = "s".into();
        tail.seq = 1;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&store.path)
            .unwrap();
        file.write_all(tail.to_value().to_string().as_bytes())
            .unwrap();
        drop(file);

        let mut reopened = Store::open("s");
        assert!(reopened.load_error().is_none());
        assert_eq!(reopened.seq(), 1);
        assert_eq!(reopened.append(Event::new(event_type::END)).unwrap().seq, 2);

        let final_store = Store::open("s");
        let sequences: Vec<i64> = final_store
            .events(0)
            .iter()
            .map(|event| event.seq)
            .collect();
        assert_eq!(sequences, vec![0, 1, 2]);
        assert!(final_store.load_error().is_none());
    }

    #[test]
    fn a_torn_unterminated_event_is_removed_without_reusing_a_sequence() {
        let _home = scratch_home("store-torn-tail-seq");
        let mut store = Store::open("s");
        store
            .append(Event::new(event_type::TEXT).saying("zero"))
            .unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&store.path)
            .unwrap();
        file.write_all(b"{\"v\":1,\"type\":\"text\",\"seq\":1")
            .unwrap();
        drop(file);

        let mut reopened = Store::open("s");
        assert!(
            reopened.load_error().is_none(),
            "a partial final write is tolerated"
        );
        assert_eq!(reopened.append(Event::new(event_type::END)).unwrap().seq, 1);

        let final_store = Store::open("s");
        let sequences: Vec<i64> = final_store
            .events(0)
            .iter()
            .map(|event| event.seq)
            .collect();
        assert_eq!(sequences, vec![0, 1]);
        assert!(final_store.load_error().is_none());
    }
}
