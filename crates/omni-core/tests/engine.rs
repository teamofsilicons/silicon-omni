//! The conductor: turns, ordering, switching, failover and what stays hot.
//!
//! Every test drives a real [`Chat`] on its own thread through the same
//! interface a client uses, against the shipped test provider.

mod harness;

use std::time::Duration;

use omni_core::chat::Change;
use omni_core::events::{AUTH, CRASH, Event, event_type};
use omni_core::providers::test as double;
use omni_core::session::Meta;
use omni_core::choose::Ask;

// ---------------------------------------------------------------- one turn

#[test]
fn a_message_gets_an_answer_and_the_turn_ends() {
    let session = harness::start("one-turn", &["test"]);
    session.send("hello");
    assert!(session.settle());
    assert_eq!(session.said(), vec!["echo: hello"]);
    assert_eq!(session.count(event_type::END), 1);
    assert_eq!(session.count(event_type::START), 1);
}

#[test]
fn the_status_says_busy_before_send_returns() {
    let session = harness::start("busy-early", &["test"]);
    double::running("test").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("hello");
    assert_eq!(
        session.snapshot().status,
        "busy",
        "no false lull for a polling loop"
    );
    double::running("test").unwrap().reply("done");
    assert!(session.settle());
    assert_eq!(session.snapshot().status, "waiting");
}

#[test]
fn tools_are_observed_in_the_order_they_happened() {
    let session = harness::start("tools", &["test"]);
    session.send("[tool:ls] please");
    assert!(session.settle());
    let kinds = session.kinds();
    let at = |what: &str| kinds.iter().position(|kind| kind == what).unwrap();
    assert!(at(event_type::START) < at(event_type::TOOL_CALL));
    assert!(at(event_type::TOOL_CALL) < at(event_type::TOOL_RESULT));
    assert!(at(event_type::TOOL_RESULT) < at(event_type::END));
}

#[test]
fn a_message_sent_mid_turn_lands_inside_it() {
    let session = harness::start("inject", &["test"]);
    double::running("test").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("first");
    session.settle_started();
    session.send("second");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        session.count(event_type::INJECTED),
        1,
        "the second joined the open turn"
    );
    assert_eq!(session.count(event_type::START), 1);
    double::running("test").unwrap().reply("done");
    assert!(session.settle());
}

// -------------------------------------------------------------- persistence

#[test]
fn the_session_file_is_the_event_log() {
    let mut session = harness::start("persisted", &["test"]);
    session.send("hello");
    assert!(session.settle());
    let seqs: Vec<i64> = session.log().into_iter().map(|event| event.seq).collect();
    assert_eq!(
        seqs,
        (0..seqs.len() as i64).collect::<Vec<_>>(),
        "seq only goes up"
    );
    session.stop();

    let reopened = omni_core::session::Store::open("persisted");
    let said: Vec<String> = reopened
        .events(0)
        .into_iter()
        .filter(|event| event.is(event_type::TEXT))
        .map(|event| event.text)
        .collect();
    assert_eq!(
        said,
        vec!["echo: hello"],
        "it survived the process that wrote it"
    );
}

// ------------------------------------------------------------------ switching

#[test]
fn raising_intelligence_moves_the_conversation_and_keeps_it() {
    let session = harness::two("switch");
    session.set(Change::Model(Ask::intelligence(0)));
    session.send("remember VIOLET-7");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "alpha");

    session.set(Change::Model(Ask::intelligence(10)));
    session.send("[recall]");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "beta");

    let recalled = session.said().pop().unwrap();
    assert!(
        recalled.contains("VIOLET-7"),
        "beta answered out of alpha's history: {recalled}"
    );
    assert_eq!(session.count(event_type::SWITCH_PROVIDER), 1);
}

#[test]
fn coming_back_replays_only_what_was_missed() {
    let session = harness::two("replay");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("one");
    assert!(session.settle());
    session.set(Change::Model(Ask::intelligence(0)));
    session.send("two");
    assert!(session.settle());
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("[recall]");
    assert!(session.settle());

    let recalled = session.said().pop().unwrap();
    assert_eq!(
        recalled.matches("one").count(),
        1,
        "beta lived through 'one'; it must not be told it again: {recalled}"
    );
    assert!(
        recalled.contains("two"),
        "but it does need alpha's turn: {recalled}"
    );
}

#[test]
fn nothing_changes_mid_turn() {
    let session = harness::two("mid-turn");
    session.set(Change::Model(Ask::intelligence(0)));
    session.send("first");
    assert!(session.settle());

    double::running("alpha").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("second");
    session.settle_started();
    session.set(Change::Model(Ask::intelligence(10))); // asked for mid-turn
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        session.snapshot().provider,
        "alpha",
        "still where the turn started"
    );

    double::running("alpha").unwrap().reply("done");
    assert!(session.until(|state| state.idle() && state.provider == "beta"));
    assert_eq!(
        session.snapshot().provider,
        "beta",
        "applied at the boundary"
    );
}

#[test]
fn a_model_change_on_one_provider_does_not_restart_it() {
    let session = harness::start("retune", &["test"]);
    session.set(Change::Model(Ask::intelligence(3)));
    session.send("hello");
    assert!(session.settle());
    let before = session.count(event_type::NEW_SESSION);

    session.set(Change::Model(Ask::intelligence(8)));
    session.send("again");
    assert!(session.settle());
    assert_eq!(
        session.count(event_type::NEW_SESSION),
        before,
        "same session, new model"
    );
    assert!(!session.notices("retune").is_empty());
    assert_eq!(session.count(event_type::SWITCH_PROVIDER), 0);
}

#[test]
fn a_provider_that_cannot_retune_is_relaunched_instead() {
    let session = harness::start("retune-refused", &["test"]);
    session.set(Change::Model(Ask::intelligence(3)));
    session.send("hello");
    assert!(session.settle());
    let (retunes, launches) = (
        session.notices("retune").len(),
        session.notices("launch").len(),
    );
    double::running("test").unwrap().set_knobs(double::Knobs {
        tunable: false,
        ..Default::default()
    });

    session.set(Change::Model(Ask::intelligence(8)));
    session.send("again");
    assert!(session.settle());
    assert_eq!(
        session.notices("retune").len(),
        retunes,
        "it refused, so nothing was retuned"
    );
    assert!(
        session.notices("launch").len() > launches,
        "it came up again instead"
    );
}

// ------------------------------------------------------------------- failover

#[test]
fn losing_a_login_moves_the_chat_to_whoever_is_left() {
    let session = harness::two("failover");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("hello");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "beta");

    double::running("beta").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("again");
    session.settle_started();
    double::running("beta").unwrap().fail(AUTH, "", true);
    assert!(
        session
            .recorded(|event| { event.is(event_type::CONFIG) && event.text == "provider_removed" })
    );
    assert!(session.settle());

    let removed = session.notices("provider_removed");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].provider, "beta");
    assert_eq!(session.snapshot().providers, vec!["alpha"]);

    session.send("[recall]");
    assert!(session.settle());
    let recalled = session.said().pop().unwrap();
    assert!(
        recalled.contains("hello"),
        "alpha picked up beta's conversation: {recalled}"
    );
}

#[test]
fn a_straggling_end_does_not_close_the_successors_turn() {
    let session = harness::two("straggler");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("hello");
    assert!(session.settle());

    double::running("beta").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("again");
    session.settle_started();
    // Every real adapter reports the failure and the end of the turn together.
    double::running("beta").unwrap().fail(AUTH, "", true);
    assert!(
        session
            .recorded(|event| { event.is(event_type::CONFIG) && event.text == "provider_removed" })
    );
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "alpha");

    double::running("alpha").unwrap().set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.send("third");
    session.settle_started();
    assert!(!session.snapshot().idle(), "alpha's turn is still open");
}

#[test]
fn turning_the_failover_off_reports_and_stops() {
    let session = harness::two("no-failover");
    session.set(Change::Autoremove(false));
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("hello");
    assert!(session.settle());
    double::running("beta").unwrap().fail(AUTH, "", true);
    assert!(session.recorded(|event| event.is(event_type::ERROR) && event.kind == AUTH));
    assert!(session.settle());
    assert!(session.notices("provider_removed").is_empty());
    assert_eq!(session.snapshot().providers, vec!["beta", "alpha"]);
}

#[test]
fn losing_the_last_provider_says_so_rather_than_hanging() {
    let session = harness::start("last-one", &["test"]);
    session.send("hello");
    assert!(session.settle());
    double::running("test").unwrap().fail(AUTH, "", true);
    assert!(session.recorded(|event| {
        event.is(event_type::ERROR) && event.error.contains("log one back in")
    }));
    assert!(session.settle());
    let blocked = session
        .log()
        .into_iter()
        .find(|event| event.is(event_type::ERROR) && event.error.contains("log one back in"));
    assert!(blocked.is_some(), "it said what to do");
    assert_eq!(session.snapshot().status, "waiting", "not stuck on busy");
}

#[test]
fn failed_turns_do_not_advance_the_providers_sync_watermark() {
    for (name, fault) in [("crash-watermark", CRASH), ("auth-watermark", AUTH)] {
        let session = harness::start(name, &["test"]);
        if fault == AUTH {
            session.set(Change::Autoremove(false));
        }
        session.send("completed");
        assert!(session.settle());
        let before = Meta::open(name).native("test").1;

        double::running("test").unwrap().set_knobs(double::Knobs {
            autoreply: false,
            ..Default::default()
        });
        session.send("this turn fails");
        session.settle_started();
        double::running("test").unwrap().fail(fault, "failed", true);
        assert!(
            session.recorded(|event| { event.is(event_type::ERROR) && event.error == "failed" })
        );
        assert!(session.settle());

        assert_eq!(
            Meta::open(name).native("test").1,
            before,
            "{fault} must leave the provider at its last completed turn"
        );
    }
}

// ------------------------------------------------------------- what is hot

#[test]
fn five_consecutive_settled_turns_keep_one_runner_and_native_session() {
    let session = harness::start("five-hot-turns", &["test"]);
    let runner = double::running("test").unwrap();
    let native = Meta::open("five-hot-turns").native("test").0;
    assert!(
        !native.is_empty(),
        "the provider has one native conversation"
    );
    assert_eq!(runner.starts(), 1, "the runner was brought up once");

    for turn in 1..=5 {
        session.send(&format!("turn {turn}"));
        assert!(session.settle(), "turn {turn} settled");
        assert_eq!(
            runner.starts(),
            1,
            "turn {turn} reused the running provider"
        );
        assert_eq!(
            Meta::open("five-hot-turns").native("test").0,
            native,
            "turn {turn} stayed in the same native conversation"
        );
    }

    assert_eq!(session.count(event_type::START), 5);
    assert_eq!(session.count(event_type::END), 5);
    assert_eq!(session.count(event_type::INJECTED), 0);
}

#[test]
fn a_provider_is_left_running_between_uses() {
    let session = harness::two("parked");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("hello");
    assert!(session.settle());
    let beta = double::running("beta").unwrap();
    assert!(beta.up());

    session.set(Change::Model(Ask::intelligence(0)));
    session.send("over here");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "alpha");
    assert!(
        beta.up(),
        "beta was set aside, not killed — coming back is free"
    );
}

#[test]
fn coming_back_to_a_parked_provider_costs_no_new_session() {
    let session = harness::two("hot");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("hello");
    assert!(session.settle());
    let beta = double::running("beta").unwrap();

    session.set(Change::Model(Ask::intelligence(0)));
    session.send("elsewhere");
    assert!(session.settle());
    assert!(beta.up(), "it stayed up while the conversation was away");

    session.set(Change::Model(Ask::intelligence(10)));
    session.send("[recall]");
    assert!(session.settle());
    let started: Vec<Event> = session
        .log()
        .into_iter()
        .filter(|event| event.is(event_type::NEW_SESSION) && event.provider == "beta")
        .collect();
    assert_eq!(started.len(), 1, "the same native session, picked back up");
    let recalled = session.said().pop().unwrap();
    assert!(
        recalled.contains("elsewhere"),
        "and told what it missed: {recalled}"
    );
}

#[test]
fn parked_deferred_catch_up_is_not_synced_before_delivery() {
    let session = harness::two("deferred-parked");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("beta heard this live");
    assert!(session.settle());
    let beta = double::running("beta").unwrap();
    beta.set_knobs(double::Knobs {
        defer: true,
        ..Default::default()
    });
    let before = Meta::open("deferred-parked").native("beta").1;

    let alpha = double::running("alpha").unwrap();
    alpha.set_knobs(double::Knobs {
        autoreply: false,
        ..Default::default()
    });
    session.set(Change::Model(Ask::intelligence(0)));
    session.send("beta has not heard this yet");
    session.settle_started();
    session.set(Change::Model(Ask::intelligence(10)));
    alpha.reply("done elsewhere");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "beta");
    assert_eq!(
        Meta::open("deferred-parked").native("beta").1,
        before,
        "agy-style catch-up is only parked in memory until the next send"
    );

    session.send("deliver the catch-up");
    assert!(session.settle());
    assert!(
        Meta::open("deferred-parked").native("beta").1 > before,
        "a completed delivery advances the watermark"
    );
}

#[test]
fn parked_deferred_provider_with_matching_tuning_is_not_relaunched() {
    let session = harness::two("deferred-nontunable-hot");
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("beta heard this live");
    assert!(session.settle());

    let beta = double::running("beta").unwrap();
    beta.set_knobs(double::Knobs {
        tunable: false,
        defer: true,
        ..Default::default()
    });
    let native = Meta::open("deferred-nontunable-hot").native("beta").0;
    assert_eq!(beta.starts(), 1);

    session.set(Change::Model(Ask::intelligence(0)));
    session.send("beta misses this turn while parked");
    assert!(session.settle());
    assert_eq!(session.snapshot().provider, "alpha");
    assert!(beta.up(), "the deferred provider stayed parked and alive");

    // Level 10 resolves to the exact same beta model and effort as before.
    // Agy cannot retune, but matching tuning means there is nothing to ask it
    // to change before deferred history is folded into this next delivery.
    session.set(Change::Model(Ask::intelligence(10)));
    session.send("deliver the deferred catch-up");
    assert!(session.settle());

    assert_eq!(session.snapshot().provider, "beta");
    assert_eq!(
        beta.starts(),
        1,
        "matching model and effort must reuse the non-tunable runner"
    );
    assert_eq!(
        Meta::open("deferred-nontunable-hot").native("beta").0,
        native,
        "the hot return kept its native conversation"
    );
}

#[test]
fn a_setting_a_running_provider_cannot_honour_replaces_it() {
    let session = harness::start("relaunch", &["test"]);
    session.set(Change::Model(Ask::intelligence(5)));
    session.send("hello");
    assert!(session.settle());
    let launches = session.notices("launch").len();

    session.set(Change::SystemPrompt("be terse".into()));
    session.send("again");
    assert!(session.settle());
    assert!(
        session.notices("launch").len() > launches,
        "a new system prompt is a new process — it cannot be told in place"
    );
}

// -------------------------------------------------------------- saying it once

#[test]
fn a_provider_says_what_it_cannot_do_once() {
    let session = harness::start("announce", &["test"]);
    session.send("one");
    assert!(session.settle());
    session.send("two");
    assert!(session.settle());
    // The double supports everything, so it announces nothing at all.
    assert!(session.notices("unsupported").is_empty());
}

#[test]
fn changing_the_setting_lets_it_be_said_again() {
    let session = harness::start("announce-again", &["test"]);
    session.send("one");
    assert!(session.settle());
    session.set(Change::Mcp(true));
    session.send("two");
    assert!(session.settle());
    let settings = session.notices("mcp");
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].extra["on"], true);
}

// ----------------------------------------------------------------- reseeding

#[test]
fn a_provider_that_forgot_the_session_is_told_everything_again() {
    let mut session = harness::start("reseed", &["test"]);
    session.send("remember VIOLET-7");
    assert!(session.settle());
    session.stop();

    // Come back to a provider that has lost the native session omni bound.
    #[allow(unused_mut)]
    let meta_path = omni_core::shared::paths::meta_file("reseed");
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    let had = meta["providers"]["test"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    meta["providers"]["test"]["id"] = serde_json::json!(format!("{}{had}", double::FORGET));
    std::fs::write(&meta_path, meta.to_string()).unwrap();

    session.reopen(&["test"]);
    session.send("[recall]");
    assert!(session.settle());
    assert_eq!(
        session.notices("reseed").len(),
        1,
        "it noticed and started clean"
    );
    let recalled = session.said().pop().unwrap();
    assert!(
        recalled.contains("VIOLET-7"),
        "and was told the whole conversation: {recalled}"
    );
}

// --------------------------------------------------------------- misbehaviour

#[test]
fn having_nothing_to_route_to_is_reported_rather_than_raised() {
    let session = harness::start("no-dial", &["test"]);
    session.set(Change::Providers(vec!["nobody".into()]));
    session.send("hello");
    assert!(session.settle());
    let blocked = session
        .log()
        .into_iter()
        .find(|event| event.is(event_type::ERROR) && event.error.contains("no answer"));
    assert!(blocked.is_some(), "{:?}", session.kinds());
    assert_eq!(session.snapshot().status, "waiting");
    assert_eq!(
        session.snapshot().queued,
        1,
        "the message is kept, not lost"
    );
}

#[test]
fn two_messages_in_a_row_are_two_turns() {
    let session = harness::start("two-turns", &["test"]);
    session.send("one");
    assert!(session.settle());
    session.send("two");
    assert!(session.settle());
    assert_eq!(session.count(event_type::START), 2);
    assert_eq!(session.count(event_type::END), 2);
    assert_eq!(session.count(event_type::INJECTED), 0);
    let starts: Vec<i64> = session
        .log()
        .into_iter()
        .filter(|event| event.is(event_type::START))
        .map(|event| event.turn)
        .collect();
    assert_eq!(starts, vec![0, 1], "turn numbers survive provider changes");
    assert_eq!(double::running("test").unwrap().sent(), vec!["one", "two"]);
}
