use super::*;
use crate::providers::test as provider;
use std::sync::Mutex;

struct Calls {
    sent: Vec<String>,
    delivery: Delivery,
    reject: usize,
    compact: Result<bool, String>,
    compactions: usize,
    stopped: bool,
    shutdown_calls: Vec<&'static str>,
}

struct Controlled(Arc<Mutex<Calls>>);

impl Runner for Controlled {
    fn name(&self) -> &str {
        provider::NAME
    }

    fn native_id(&self) -> String {
        "original-native".into()
    }

    fn start(&mut self, _: &str, _: &[Event]) -> Result<(), String> {
        Ok(())
    }

    fn send(&mut self, text: &str) -> Result<Delivery, String> {
        let mut calls = self.0.lock().unwrap();
        calls.sent.push(text.into());
        if calls.reject > 0 {
            calls.reject -= 1;
            return Err("context_window_exceeded".into());
        }
        Ok(calls.delivery)
    }

    fn compact(&mut self) -> Result<bool, String> {
        let mut calls = self.0.lock().unwrap();
        calls.compactions += 1;
        calls.compact.clone()
    }

    fn interrupt(&mut self) {
        self.0.lock().unwrap().shutdown_calls.push("interrupt");
    }

    fn stop(&mut self) {
        let mut calls = self.0.lock().unwrap();
        calls.shutdown_calls.push("stop");
        calls.stopped = true;
    }

    fn alive(&self) -> bool {
        !self.0.lock().unwrap().stopped
    }
}

fn controlled(chat: &mut Chat, compact: Result<bool, String>, reject: usize) -> Arc<Mutex<Calls>> {
    let calls = Arc::new(Mutex::new(Calls {
        sent: Vec::new(),
        delivery: Delivery::Immediate,
        reject,
        compact,
        compactions: 0,
        stopped: false,
        shutdown_calls: Vec::new(),
    }));
    if let Some(mut runner) = chat.runner.take() {
        runner.stop();
    }
    chat.runner = Some(Box::new(Controlled(calls.clone())));
    chat.runner_epoch = None;
    chat.current = Some(chat.signature(&chat.rung().unwrap()));
    calls
}

fn open(
    name: &str,
    compact: Result<bool, String>,
    reject: usize,
) -> (Chat, Handle, Arc<Mutex<Calls>>) {
    let names = provider::install(&[], &[]);
    provider::prepare(provider::NAME).set_knobs(provider::Knobs {
        autoreply: false,
        ..provider::Knobs::default()
    });
    let (mut chat, handle) = Chat::open(name, names, Arc::new(|_| {}));
    let calls = controlled(&mut chat, compact, reject);
    (chat, handle, calls)
}

fn end(chat: &mut Chat) {
    chat.absorb(Event::new(event_type::END).from(provider::NAME));
}

fn limit(chat: &mut Chat) {
    chat.absorb(Event::failure(CONTEXT_LIMIT, "context_window_exceeded").from(provider::NAME));
    end(chat);
}

fn stage(chat: &Chat) -> Option<RecoveryStage> {
    chat.recovery.as_ref().map(|recovery| recovery.stage)
}

#[test]
fn native_compaction_waits_for_end_then_retries_once_before_queued_work() {
    let _home = crate::testing::scratch_home("recovery-native");
    let (mut chat, handle, calls) = open("s", Ok(true), 0);
    chat.dispatch("original ask".into());
    limit(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Compacting));
    assert!(chat.in_turn);
    assert!(chat.compaction_deadline.is_some());

    chat.dispatch("queued work".into());
    chat.absorb(Event::new(event_type::TOOL_RESULT).from(provider::NAME));
    assert_eq!(calls.lock().unwrap().sent, ["original ask"]);
    assert_eq!(handle.snapshot().queued, 1);
    assert_eq!(handle.snapshot().status, BUSY);

    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Retrying));
    assert!(chat.compaction_deadline.is_none());
    assert_eq!(calls.lock().unwrap().sent, ["original ask", "original ask"]);
    assert_eq!(chat.outbox.front().unwrap().text, "queued work");
    end(&mut chat);
    assert_eq!(stage(&chat), None);
    assert_eq!(
        calls.lock().unwrap().sent,
        ["original ask", "original ask", "queued work"]
    );
    assert_eq!(calls.lock().unwrap().compactions, 1);
    end(&mut chat);
    assert!(handle.snapshot().idle());
    assert!(Meta::open("s").pending().is_empty());
}

#[test]
fn unsuccessful_compaction_falls_back_without_retrying_compaction() {
    let _home = crate::testing::scratch_home("recovery-compact-failure");
    for (name, compact) in [
        ("unsupported", Ok(false)),
        ("rejected", Err("method unavailable".into())),
        ("failed", Ok(true)),
    ] {
        let (mut chat, _, calls) = open(name, compact, 0);
        chat.dispatch("pending work".into());
        limit(&mut chat);
        if stage(&chat) == Some(RecoveryStage::Compacting) {
            chat.absorb(Event::failure(UNAVAILABLE, "compaction unavailable").from(provider::NAME));
            end(&mut chat);
        }
        assert_eq!(stage(&chat), Some(RecoveryStage::Handoff));
        assert_eq!(calls.lock().unwrap().compactions, 1);
        assert!(calls.lock().unwrap().stopped);
        assert_eq!(provider::running(provider::NAME).unwrap().sent().len(), 1);
        assert!(!chat.benched.contains(provider::NAME));
    }
}

#[test]
fn a_second_context_failure_after_compaction_uses_the_handoff() {
    let _home = crate::testing::scratch_home("recovery-retry-once");
    let (mut chat, _, calls) = open("s", Ok(true), 1);
    chat.dispatch("too much context".into());
    assert_eq!(stage(&chat), Some(RecoveryStage::Compacting));
    calls.lock().unwrap().reject = 1;
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Handoff));
    assert_eq!(
        calls.lock().unwrap().sent,
        ["too much context", "too much context"]
    );
    assert_eq!(calls.lock().unwrap().compactions, 1);
}

#[test]
fn settings_changed_during_compaction_apply_before_either_retry_path() {
    let _home = crate::testing::scratch_home("recovery-settings-boundary");
    for rejected in [false, true] {
        let (mut chat, _, calls) = open(
            if rejected { "rejected" } else { "delivered" },
            Ok(true),
            usize::from(rejected),
        );
        chat.dispatch("finish the original task".into());
        if !rejected {
            limit(&mut chat);
        }
        chat.apply(Change::Model(Ask::intelligence(9))).unwrap();
        assert!(chat.stale());
        assert_eq!(calls.lock().unwrap().sent.len(), 1);
        end(&mut chat);
        assert_eq!(stage(&chat), Some(RecoveryStage::Retrying));
        assert!(!chat.stale());
        assert!(chat.in_turn);
        assert_eq!(
            provider::running(provider::NAME).unwrap().sent(),
            ["finish the original task"]
        );
        end(&mut chat);
        assert_eq!(stage(&chat), None);
        assert!(chat.outbox.is_empty());
    }
}

#[test]
fn an_oversized_pending_message_after_continuation_stops_without_another_rotation() {
    let _home = crate::testing::scratch_home("recovery-oversized-input");
    let (mut chat, handle, _) = open("s", Ok(false), 1);
    chat.dispatch("oversized pending input".into());
    assert_eq!(stage(&chat), Some(RecoveryStage::Handoff));
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Continuing));
    let live = provider::running(provider::NAME).unwrap();
    let starts = live.starts();
    let calls = controlled(&mut chat, Ok(true), usize::MAX);
    end(&mut chat);
    assert_eq!(stage(&chat), None);
    assert_eq!(calls.lock().unwrap().sent, ["oversized pending input"]);
    assert_eq!(calls.lock().unwrap().compactions, 0);
    assert!(calls.lock().unwrap().stopped);
    assert_eq!(live.starts(), starts);
    assert_eq!(Meta::open("s").pending()[0].text, "oversized pending input");
    assert_eq!(handle.snapshot().status, WAITING);
    assert_eq!(handle.snapshot().queued, 1);
    assert!(
        chat.store
            .events(0)
            .iter()
            .any(|event| event.extra.get("recovery_failed") == Some(&json!(true)))
    );
}

#[test]
fn cold_interrupted_retry_and_handoff_preserve_text_and_wait_for_the_save_turn() {
    let _home = crate::testing::scratch_home("recovery-cold-interrupted");
    for initial in [RecoveryStage::Retrying, RecoveryStage::Handoff] {
        for interrupted in [false, true] {
            let name = format!("{initial:?}-{interrupted}");
            let (mut chat, _, _) = open(&name, Ok(initial == RecoveryStage::Retrying), 0);
            chat.dispatch("original task with details".into());
            limit(&mut chat);
            if initial == RecoveryStage::Retrying {
                end(&mut chat);
            }
            assert_eq!(stage(&chat), Some(initial));
            let opening = chat
                .store
                .events(0)
                .into_iter()
                .rev()
                .find(|event| event.is(event_type::START))
                .unwrap()
                .text;
            chat.absorb(
                Event::new(event_type::TEXT).saying("partial details, saving is still pending"),
            );
            if interrupted {
                chat.shutdown();
            }
            drop(chat);

            let (mut restored, _) =
                Chat::open(&name, vec![provider::NAME.into()], Arc::new(|_| {}));
            restored.launch_once();
            assert_eq!(stage(&restored), Some(RecoveryStage::Handoff));
            assert!(restored.in_turn);
            let live = provider::running(provider::NAME).unwrap();
            let replay = live.sent();
            assert_eq!(replay.len(), 1);
            assert!(replay[0].contains("original task with details"));
            assert!(
                replay[0].contains(&opening),
                "keep the entire unsaved handoff"
            );
            assert!(replay[0].contains("partial details, saving is still pending"));
            assert!(!replay[0].contains(&restored.context_recovery.new_session_message));
            restored.absorb(Event::new(event_type::TEXT).saying("details saved"));
            assert_eq!(stage(&restored), Some(RecoveryStage::Handoff));
            end(&mut restored);
            assert_eq!(stage(&restored), Some(RecoveryStage::Continuing));
            assert_eq!(
                live.sent(),
                [restored.context_recovery.new_session_message.clone()]
            );
        }
    }
}

#[test]
fn recovery_metadata_failure_stops_before_compaction_or_advancing_the_handoff() {
    let home = crate::testing::scratch_home("recovery-metadata-failure");
    let blocker = home.path.join("regular-file");
    std::fs::write(&blocker, "not a directory").unwrap();
    for handoff in [false, true] {
        let name = if handoff { "handoff" } else { "compact" };
        let (mut chat, handle, calls) = open(name, Ok(!handoff), 0);
        chat.dispatch("preserve this task".into());
        if handoff {
            limit(&mut chat);
            chat.absorb(Event::new(event_type::TEXT).saying("saved work"));
        }
        let before = Meta::open(name).get(RECOVERY).cloned();
        let history_start = chat.history_start;
        let starts = provider::prepare(provider::NAME).starts();
        chat.meta.path = blocker.join("metadata.json");
        if handoff {
            end(&mut chat);
        } else {
            limit(&mut chat);
        }
        assert!(chat.terminal);
        assert_eq!(handle.snapshot().status, STOPPED);
        assert!(calls.lock().unwrap().stopped);
        assert_eq!(calls.lock().unwrap().compactions, usize::from(handoff));
        assert_eq!(chat.history_start, history_start);
        assert_eq!(Meta::open(name).get(RECOVERY).cloned(), before);
        assert_eq!(provider::prepare(provider::NAME).starts(), starts);
    }
}

#[test]
fn an_expired_compaction_deadline_falls_back_before_queued_chatter_or_stop() {
    let _home = crate::testing::scratch_home("recovery-expired-deadline");
    let (mut chat, _, calls) = open("s", Ok(true), 0);
    chat.dispatch("original task".into());
    limit(&mut chat);
    chat.launched_once = true;
    chat.compaction_deadline = Some(std::time::Instant::now());
    chat.post
        .send(Wake::Heard(Box::new(Event::config("chatter"))))
        .unwrap();
    chat.post.send(Wake::Stop).unwrap();
    chat.run();
    let persisted: Recovery =
        serde_json::from_value(Meta::open("s").get(RECOVERY).unwrap().clone()).unwrap();
    assert_eq!(persisted.stage, RecoveryStage::Handoff);
    assert!(calls.lock().unwrap().stopped);
    assert_eq!(calls.lock().unwrap().shutdown_calls, ["interrupt", "stop"]);
    let sent = provider::running(provider::NAME).unwrap().sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("original task"));
}

#[test]
fn harmless_stderr_after_text_does_not_fail_compaction_or_the_handoff() {
    let _home = crate::testing::scratch_home("recovery-stderr-warning");
    for compact in [false, true] {
        let name = if compact { "compact" } else { "handoff" };
        let (mut chat, _, _) = open(name, Ok(compact), 0);
        chat.dispatch("original task".into());
        limit(&mut chat);
        let before = stage(&chat);
        chat.absorb(Event::new(event_type::TEXT).saying("details saved"));
        chat.absorb(Event::failure("stderr", "IAM warning").from(provider::NAME));
        assert_eq!(stage(&chat), before);
        assert!(chat.turn_failed.is_none());
        end(&mut chat);
        assert_eq!(
            stage(&chat),
            Some(if compact {
                RecoveryStage::Retrying
            } else {
                RecoveryStage::Continuing
            })
        );
        assert!(chat.in_turn);
    }
}

#[test]
fn a_refused_synthesized_retry_leaves_original_pending_work_first_to_resume() {
    let _home = crate::testing::scratch_home("recovery-synthesized-retry-fifo");
    let (mut chat, _, calls) = open("s", Ok(true), 0);
    chat.dispatch("task A".into());
    calls.lock().unwrap().reject = 1;
    chat.dispatch("pending B".into());
    assert_eq!(stage(&chat), None);
    assert_eq!(chat.outbox.front().unwrap().text, "pending B");
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Compacting));
    calls.lock().unwrap().reject = 1;
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Handoff));
    assert_eq!(
        calls.lock().unwrap().sent,
        ["task A", "pending B", "task A"]
    );
    assert_eq!(calls.lock().unwrap().compactions, 1);
    assert_eq!(chat.outbox.len(), 1);
    assert_eq!(chat.outbox.front().unwrap().text, "pending B");
    assert_eq!(Meta::open("s").pending()[0].text, "pending B");
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Continuing));
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Resuming));
    assert_eq!(
        provider::running(provider::NAME).unwrap().sent(),
        [
            chat.context_recovery.new_session_message.clone(),
            "pending B".into()
        ]
    );
    end(&mut chat);
    assert_eq!(stage(&chat), None);
    assert!(chat.outbox.is_empty());
    assert!(Meta::open("s").pending().is_empty());
}

#[test]
fn a_cold_successful_recovery_end_launches_before_delivering_queued_work() {
    let _home = crate::testing::scratch_home("recovery-cold-completed-end");
    for initial in [
        RecoveryStage::Retrying,
        RecoveryStage::Continuing,
        RecoveryStage::Resuming,
    ] {
        let name = format!("{initial:?}");
        let (mut chat, _, _) = open(&name, Ok(initial == RecoveryStage::Retrying), 0);
        chat.dispatch("original task".into());
        limit(&mut chat);
        end(&mut chat); // finish compaction or the handoff
        if initial == RecoveryStage::Resuming {
            chat.dispatch("first queued task".into());
            end(&mut chat); // finish the continuation, begin queued work
        }
        assert_eq!(stage(&chat), Some(initial));
        chat.dispatch("next queued task".into());
        chat.absorb(Event::new(event_type::TEXT).saying("turn completed"));
        // The provider END reached disk, but the daemon exited before applying
        // recovery_ended and saving the next recovery state.
        chat.record(Event::new(event_type::END)).unwrap();
        drop(chat);

        let (mut restored, handle) =
            Chat::open(&name, vec![provider::NAME.into()], Arc::new(|_| {}));
        restored.launch_once();
        assert!(
            restored.in_turn,
            "{initial:?}: completed recovery must resume accepted queued work"
        );
        assert_eq!(
            provider::running(provider::NAME).unwrap().sent(),
            ["next queued task"],
            "{initial:?}"
        );
        assert!(restored.outbox.is_empty());
        assert!(Meta::open(&name).pending().is_empty());
        end(&mut restored);
        assert_eq!(stage(&restored), None);
        assert!(handle.snapshot().idle());
    }
}

#[test]
fn an_unconfirmed_synthesized_retry_is_retired_before_handoff_requeues_echoes() {
    let _home = crate::testing::scratch_home("recovery-superseded-echo");
    let (mut chat, _, calls) = open("s", Ok(true), 0);
    chat.dispatch("original task".into());
    limit(&mut chat);
    chat.dispatch("newer queued task".into());
    calls.lock().unwrap().delivery = Delivery::Echoed;
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Retrying));
    let retired = chat.recovery.as_ref().unwrap().message_id.clone();
    assert_eq!(chat.awaiting.front().unwrap().id, retired);

    limit(&mut chat); // the retry failed before the provider echoed it
    assert_eq!(stage(&chat), Some(RecoveryStage::Handoff));
    assert!(chat.awaiting.is_empty());
    assert!(chat.outbox.iter().all(|message| message.id != retired));
    assert!(
        Meta::open("s")
            .pending()
            .iter()
            .all(|message| message.id != retired)
    );
    assert_eq!(chat.outbox.len(), 1);
    assert_eq!(chat.outbox.front().unwrap().text, "newer queued task");
    end(&mut chat);
    end(&mut chat);
    assert_eq!(stage(&chat), Some(RecoveryStage::Resuming));
    assert_eq!(
        provider::running(provider::NAME).unwrap().sent(),
        [
            chat.context_recovery.new_session_message.clone(),
            "newer queued task".into()
        ]
    );
    end(&mut chat);
    assert_eq!(stage(&chat), None);
    assert!(Meta::open("s").pending().is_empty());
}

#[test]
fn invalid_recovery_metadata_refuses_to_discard_or_replay_saved_work() {
    let _home = crate::testing::scratch_home("recovery-corrupt-state");
    for (index, (key, value)) in [
        (RECOVERY, json!("not a recovery state")),
        (
            RECOVERY,
            json!({"stage": "Unknown", "message_id": "id", "retry": ""}),
        ),
        (
            RECOVERY,
            json!({"stage": "Retrying", "message_id": "", "retry": ""}),
        ),
        (HISTORY_START, json!(-1)),
        (HISTORY_START, json!(1000)),
        (HISTORY_START, json!("1")),
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("invalid-{index}");
        let (mut chat, _, _) = open(&name, Ok(true), 0);
        chat.dispatch("saved conversation".into());
        chat.enqueue("accepted pending work".into()).unwrap();
        chat.meta.set(key, value).unwrap();
        let path = chat.meta.path.clone();
        let metadata = std::fs::read(&path).unwrap();
        let log = chat.store.events(0);
        let starts = provider::prepare(provider::NAME).starts();
        drop(chat);

        let (restored, handle) = Chat::open(&name, vec![provider::NAME.into()], Arc::new(|_| {}));
        restored.run();
        assert_eq!(handle.snapshot().status, STOPPED);
        assert_eq!(std::fs::read(path).unwrap(), metadata);
        assert_eq!(json!(Store::open(&name).events(0)), json!(log));
        assert_eq!(Meta::open(&name).pending()[0].text, "accepted pending work");
        assert_eq!(provider::prepare(provider::NAME).starts(), starts);
    }
}
