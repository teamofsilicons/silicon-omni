//! Context overflow recovery must leave the durable conversation intact.

#[allow(dead_code)]
mod harness;

use omni_core::chat::{Change, ContextRecovery};
use omni_core::events::event_type;
use omni_core::providers::test as double;
use omni_core::session::{Meta, Store};

#[test]
fn unsupported_compaction_hands_off_text_then_starts_a_clean_session() {
    let mut session = harness::start("context-handoff", &["test"]);
    session.send("Remember VIOLET-7 [tool:Read]");
    assert!(session.settle());
    let live = double::running("test").unwrap();
    live.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    let starts = live.starts();
    session.send("Finish the pending work");
    assert!(session.settle_started());
    live.fail("context_limit", "context_window_exceeded", true);

    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 1 && !live.sent().is_empty()
    }));
    let handoff = live.sent();
    assert_eq!(handoff.len(), 1, "history goes in the first message");
    assert!(live.given().is_empty(), "the full native log is not seeded");
    assert!(!live.resumed(), "a full native session cannot be resumed");
    assert!(handoff[0].contains("USER: Remember VIOLET-7"));
    assert!(handoff[0].contains("ASSISTANT: echo: Remember VIOLET-7"));
    assert!(handoff[0].contains("Finish the pending work"));
    assert!(handoff[0].ends_with(&ContextRecovery::default().limit_message));
    assert!(!handoff[0].contains("[Read result:"));
    assert!(!handoff[0].contains("ran Read"));
    assert_eq!(session.snapshot().providers, ["test"]);

    live.reply("Saved pending work to HANDOFF.md");
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 2 && !live.sent().is_empty()
    }));
    assert_eq!(
        live.sent(),
        [ContextRecovery::default().new_session_message]
    );
    assert!(!live.resumed());
    live.reply("Pending work completed");
    assert!(session.settle());
    assert_eq!(
        live.starts(),
        starts + 2,
        "the continuation does not rotate again"
    );
    assert!(session.notices("provider_removed").is_empty());
    session.stop();

    let log = Store::open("context-handoff").events(0);
    assert!(log.iter().any(|event| event.is(event_type::TOOL_CALL)));
    assert!(
        log.iter()
            .any(|event| { event.is(event_type::TOOL_RESULT) && event.result == "ran Read" })
    );
    for text in [
        "Finish the pending work",
        "Saved pending work to HANDOFF.md",
        "Pending work completed",
    ] {
        assert!(log.iter().any(|event| event.text == text), "{text}");
    }
    assert!(log.iter().all(|event| event.session == "context-handoff"));
    assert!(log.windows(2).all(|pair| pair[1].seq == pair[0].seq + 1));
}

#[test]
fn recovery_uses_configured_texts_and_keeps_system_prompts() {
    let session = harness::start("context-config", &["test"]);
    let messages = ContextRecovery {
        limit_message: "Save a handoff before this turn ends.".into(),
        new_session_message: "Read HANDOFF.md and continue.".into(),
        transcript_header: "Conversation text for the handoff:".into(),
    };
    session
        .handle
        .configure(vec![
            Change::SystemPrompt("Persist pending work in HANDOFF.md.".into()),
            Change::AppendSystemPrompt("Keep the project conventions.".into()),
            Change::ContextRecovery(messages.clone()),
        ])
        .unwrap();
    session.send("Remember AMBER-9");
    assert!(session.settle());
    let live = double::running("test").unwrap();
    live.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    let starts = live.starts();
    session.send("Continue");
    assert!(session.settle_started());
    live.fail("context_limit", "Prompt is too long", true);
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 1 && !live.sent().is_empty()
    }));
    let handoff = live.sent();
    assert!(handoff[0].starts_with(&messages.transcript_header));
    assert!(handoff[0].ends_with(&messages.limit_message));
    live.reply("Handoff saved");
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 2 && !live.sent().is_empty()
    }));
    assert_eq!(live.sent(), [messages.new_session_message.clone()]);
    live.reply("Finished");
    assert!(session.settle());

    let meta = Meta::open("context-config");
    assert_eq!(
        meta.setting_str("system_prompt").as_deref(),
        Some("Persist pending work in HANDOFF.md.")
    );
    assert_eq!(
        meta.setting_str("append_system_prompt").as_deref(),
        Some("Keep the project conventions.")
    );
    assert_eq!(
        meta.setting("context_recovery"),
        Some(&serde_json::to_value(messages).unwrap())
    );
}

#[test]
fn new_user_messages_wait_until_both_recovery_turns_finish() {
    let session = harness::start("context-queued", &["test"]);
    let live = double::running("test").unwrap();
    live.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    let starts = live.starts();
    session.send("Complete the original work");
    assert!(session.settle_started());
    live.fail("context_limit", "context_window_exceeded", true);
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 1 && !live.sent().is_empty()
    }));

    session.send("First queued request");
    assert!(session.until(|state| state.queued == 1));
    assert_eq!(live.sent().len(), 1, "new work must not join the handoff");
    live.reply("Handoff saved");
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 2 && !live.sent().is_empty()
    }));
    session.send("Second queued request");
    assert!(session.until(|state| state.queued == 2));
    assert_eq!(
        live.sent(),
        [ContextRecovery::default().new_session_message],
        "new work must also wait through the continuation"
    );

    live.reply("Original pending work finished");
    assert!(session.until(|state| state.in_turn && state.queued == 1 && live.sent().len() == 2));
    assert_eq!(live.sent()[1], "First queued request");
    assert_eq!(session.snapshot().queued, 1);
    live.reply("First queued request finished");
    assert!(session.until(|state| state.in_turn && live.sent().len() == 3));
    assert_eq!(
        &live.sent()[1..],
        ["First queued request", "Second queued request"],
        "queued requests are delivered in order, exactly once"
    );
    live.reply("Second queued request finished");
    assert!(session.settle());
    assert_eq!(session.snapshot().queued, 0);
    assert_eq!(live.starts(), starts + 2);
    for text in ["First queued request", "Second queued request"] {
        assert_eq!(
            session
                .log()
                .iter()
                .filter(|event| event.text == text)
                .count(),
            1
        );
    }
}

#[test]
fn a_context_failure_during_handoff_stops_after_one_replacement() {
    let session = harness::start("context-handoff-failed", &["test"]);
    let live = double::running("test").unwrap();
    live.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    let starts = live.starts();
    session.send("A conversation whose text alone fills the context window");
    assert!(session.settle_started());
    live.fail("context_limit", "context_window_exceeded", true);
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 1 && !live.sent().is_empty()
    }));
    live.fail("context_limit", "Prompt is too long", true);
    assert!(session.recorded(|event| {
        event.is(event_type::ERROR)
            && event.extra.get("recovery_failed") == Some(&serde_json::json!(true))
    }));
    assert!(session.settle());
    assert_eq!(live.starts(), starts + 1, "failed handoffs cannot loop");
    assert!(!live.up(), "the overflowing handoff session was stopped");
    assert_eq!(session.snapshot().queued, 0);
    assert!(session.notices("provider_removed").is_empty());
    assert!(
        Meta::open("context-handoff-failed")
            .get("context_recovery_state")
            .is_none()
    );
}

#[test]
fn a_generic_crash_with_context_details_still_recovers() {
    let session = harness::start("context-generic-error", &["test"]);
    let live = double::running("test").unwrap();
    live.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    let starts = live.starts();
    session.send("yo");
    assert!(session.settle_started());
    live.fail(
        omni_core::events::CRASH,
        "context_window_exceeded: Codex ran out of room in the model's context window.",
        true,
    );
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 1 && !live.sent().is_empty()
    }));
    assert!(session.log().iter().any(|event| {
        event.is(event_type::ERROR)
            && event.kind == "context_limit"
            && event.error.contains("Codex ran out of room")
    }));
    assert!(live.sent()[0].contains("USER: yo"));
    live.reply("Handoff saved");
    assert!(session.until(|state| {
        state.in_turn && live.starts() == starts + 2 && !live.sent().is_empty()
    }));
    live.reply("Pending work checked");
    assert!(session.settle());
    assert_eq!(session.snapshot().providers, ["test"]);
    assert!(session.notices("provider_removed").is_empty());
}
