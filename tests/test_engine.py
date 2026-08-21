"""Test group: the chat engine — turns, status, injection, boundaries, switching.

Driven entirely by the fake provider, so it never touches a real CLI.
"""

import time

import pytest

from omni import providers
from omni.chat import Chat
from omni.events import Event

from omni.providers.test import LIVE


def settle(chat, want="waiting", timeout=5.0):
    """Wait for the conductor to go quiet."""
    end = time.time() + timeout
    while time.time() < end:
        if chat.status == want and chat.inbox.empty():
            return True
        time.sleep(0.005)
    return False


@pytest.fixture
def chat(two_providers):
    """Level 0 is alpha's weakest rung, level 10 is beta's strongest."""
    chat = Chat("engine", two_providers)
    chat.intelligence(0)
    yield chat
    chat.stop()


def test_status_walks_idle_waiting_busy_stopped(chat):
    assert chat.status == "idle"
    chat.start()
    assert settle(chat)
    chat.send("hi")
    assert chat.status == "busy"
    assert settle(chat)
    chat.stop()
    assert chat.status == "stopped"


def test_a_turn_is_recorded_as_events(chat):
    seen = []
    chat.on_event(seen.append)
    chat.start()
    chat.send("hello")
    assert settle(chat)
    types = [e.type for e in seen]
    assert Event.START in types and Event.TEXT in types and Event.END in types
    assert [e.text for e in seen if e.type == Event.TEXT] == ["echo: hello"]


def test_history_survives_a_reload(chat):
    chat.start()
    chat.send("remember me")
    assert settle(chat)
    chat.stop()
    again = Chat("engine", ["alpha", "beta"])
    texts = [e.text for e in again.store.history()]
    assert "remember me" in texts and "echo: remember me" in texts
    again.stop()


def test_message_sent_mid_turn_is_injected(chat):
    chat.start()
    assert settle(chat)
    LIVE["alpha"].autoreply = False
    try:
        chat.send("first")
        assert settle(chat, want="busy")
        chat.send("second")
        time.sleep(0.05)
        kinds = [e.type for e in chat.store.events()]
        assert Event.INJECTED in kinds
        assert LIVE["alpha"].sent == ["first", "second"]
    finally:
        LIVE["alpha"].autoreply = True


def test_intelligence_change_waits_for_the_turn_to_end(chat):
    chat.start()
    assert settle(chat)
    assert chat.runner.config.model == "alpha-small"
    LIVE["alpha"].autoreply = False
    chat.send("slow one")
    assert settle(chat, want="busy")
    chat.intelligence(10)
    time.sleep(0.05)
    assert chat.runner.config.model == "alpha-small", "changed mid-turn"
    runner = LIVE["alpha"]
    runner.autoreply = True
    runner.reply("done")
    assert settle(chat)
    assert chat.runner.config.model == "beta-big"


def test_switching_provider_seeds_the_new_one(chat):
    chat.start()
    chat.send("one")
    assert settle(chat)
    chat.intelligence(10)
    chat.send("two")
    assert settle(chat)
    beta = LIVE["beta"]
    assert chat.runner.name == "beta"
    seeded = [e.text for e in beta.given]
    assert "one" in seeded and "echo: one" in seeded
    assert "two" not in seeded, "the live message is sent, not seeded"
    assert beta.sent == ["two"]
    assert Event.SWITCH_PROVIDER in [e.type for e in chat.store.events()]


def test_coming_back_only_replays_what_was_missed(chat):
    chat.start()
    chat.send("one")
    assert settle(chat)
    alpha_native = chat.runner.native_id
    chat.intelligence(10)
    chat.send("two")
    assert settle(chat)
    chat.intelligence(0)
    chat.send("three")
    assert settle(chat)
    alpha = LIVE["alpha"]
    assert chat.runner.name == "alpha"
    assert alpha.resumed and alpha.native_id == alpha_native, "should resume its own session"
    seeded = [e.text for e in alpha.given]
    assert "two" in seeded and "echo: two" in seeded, "the part it missed"
    assert "one" not in seeded, "it already knew this"
    assert alpha.sent == ["three"]


def test_level_maps_across_every_available_provider(two_providers):
    from omni.intelligence import table

    rungs = table(two_providers)
    assert rungs[0]["provider"] == "alpha" and rungs[0]["model"] == "alpha-small"
    assert rungs[10]["provider"] == "beta" and rungs[10]["model"] == "beta-big"
    assert [r["provider"] for r in rungs.values()].count("alpha") >= 1


def test_new_session_event_fires_once_per_provider(chat):
    chat.start()
    chat.send("one")
    assert settle(chat)
    chat.intelligence(10)
    chat.send("two")
    assert settle(chat)
    chat.intelligence(0)
    chat.send("three")
    assert settle(chat)
    fresh = [e.extra["native"] for e in chat.store.events() if e.type == Event.NEW_SESSION]
    assert len(fresh) == 2, fresh


def test_logs_see_more_than_on_event(chat):
    events, logged = [], []
    chat.on_event(events.append)
    chat.logs(logged.append)
    chat.start()
    chat.intelligence(3)
    chat.send("hi")
    assert settle(chat)
    assert any(e.type == Event.CONFIG for e in logged)
    assert len(logged) == len(events)


def test_a_broken_handler_cannot_kill_the_chat(chat):
    problems = []
    chat.logs(lambda e: problems.append(e) if e.kind == "handler" else None)

    @chat.on_event
    def bad(event):
        raise ValueError("boom")

    chat.start()
    chat.send("hi")
    assert settle(chat)
    assert problems
    assert chat.status == "waiting"


def test_a_provider_that_lost_its_session_is_told_everything_again(chat):
    chat.start()
    chat.send("one")
    assert settle(chat)
    chat.stop()

    # the provider forgot: its id no longer resolves
    from omni.session import Meta

    meta = Meta("engine")
    meta.bind("alpha", "gone-1")
    again = Chat("engine", ["alpha", "beta"])
    again.intelligence(0)
    again.start()
    again.send("two")
    assert settle(again)
    try:
        alpha = LIVE["alpha"]
        seeded = [e.text for e in alpha.given]
        assert "one" in seeded and "echo: one" in seeded, "it should be told the whole story"
        assert alpha.native_id != "gone-1"
        assert "reseed" in [e.text for e in again.store.events() if e.type == Event.CONFIG]
    finally:
        again.stop()


def test_stopping_mid_turn_still_records_what_arrived(chat):
    chat.start()
    assert settle(chat)
    LIVE["alpha"].autoreply = False
    chat.send("slow")
    assert settle(chat, want="busy")
    runner = LIVE["alpha"]
    runner.reply("the last thing it said")  # lands as stop is being asked for
    chat.stop()
    texts = [e.text for e in chat.store.events() if e.type == Event.TEXT]
    assert "the last thing it said" in texts
    assert chat.status == "stopped"


def test_a_stopped_chat_refuses_new_messages(chat):
    chat.start()
    assert settle(chat)
    chat.stop()
    with pytest.raises(RuntimeError):
        chat.send("too late")


def test_stopping_from_inside_a_handler_does_not_hang(chat):
    @chat.on_event
    def bail(event):
        if event.type == Event.END:
            chat.stop()

    chat.start()
    chat.send("hi")
    end = time.time() + 10
    while time.time() < end and chat.status != "stopped":
        time.sleep(0.01)
    assert chat.status == "stopped"


def test_every_replaced_runner_is_stopped(chat):
    """A dropped reference is not a stopped process — that leaks a live CLI."""
    seen = []
    for name in ("alpha", "beta"):
        runner_cls = providers.classes(name)[1]
        began = runner_cls.start

        def remember(self, native_id="", history=None, began=began):
            began(self, native_id, history)
            seen.append(self)

        runner_cls.start = remember

    chat.start()
    for level in (10, 0) * 6:
        chat.intelligence(level)
        chat.send(f"turn {level}")
        assert settle(chat)
    assert len(seen) > 6, "the dial should have rebuilt several times"
    assert all(not r.alive for r in seen[:-1]), "a replaced runner was dropped but never stopped"
    assert seen[-1] is chat.runner


def test_a_provider_is_not_marked_caught_up_until_it_has_the_history(chat, two_providers):
    """agy only takes its seed with the next message; dropping it must not lose the history."""
    from omni import providers

    providers.classes("beta")[1].defer = True
    chat.start()
    chat.send("one")
    assert settle(chat)

    # change the dial mid-turn, so beta is launched at the boundary with nothing to send
    LIVE["alpha"].autoreply = False
    chat.send("two")
    assert settle(chat, want="busy")
    chat.intelligence(10)
    LIVE["alpha"].autoreply = True
    LIVE["alpha"].reply("done")
    assert settle(chat)
    assert chat.runner.name == "beta" and not chat.runner.seeded

    # ...and change it again before beta ever hears anything
    chat.intelligence(0)
    chat.send("three")
    assert settle(chat)
    chat.intelligence(10)
    chat.send("four")
    assert settle(chat)

    seeded = [e.text for e in LIVE["beta"].given]
    assert "one" in seeded and "two" in seeded, "beta never took the first seed, so it gets it again"


def test_a_chat_that_never_started_still_gives_the_session_id_back(two_providers):
    from omni.session import Lock

    chat = Chat("unstarted", two_providers)
    chat.stop()
    Lock("unstarted").acquire().release()  # would raise SessionBusy if still held


def test_a_provider_that_will_not_start_does_not_swallow_the_message(chat, two_providers):
    from omni import providers

    runner_cls = providers.classes("alpha")[1]
    original = runner_cls.start
    runner_cls.start = lambda self, native_id="", history=None: (_ for _ in ()).throw(OSError("no such CLI"))
    try:
        chat.start()
        chat.send("please remember this")
        end = time.time() + 5
        while time.time() < end and chat.status == "busy":
            time.sleep(0.01)
        assert chat.status == "waiting", "a failed launch must not hang in busy"
        assert chat.outbox == ["please remember this"], "the message is still queued"
        assert any(e.type == Event.ERROR for e in chat.store.events())
    finally:
        runner_cls.start = original

    # ...and it goes out once something can carry it
    chat.send("and this")
    assert settle(chat)
    assert LIVE["alpha"].sent == ["please remember this", "and this"]


def test_a_send_that_fails_leaves_the_message_queued(chat):
    chat.start()
    assert settle(chat)
    runner = LIVE["alpha"]
    runner.send = lambda text: (_ for _ in ()).throw(BrokenPipeError("gone"))
    chat.send("must not vanish")
    end = time.time() + 5
    while time.time() < end and chat.status == "busy":
        time.sleep(0.01)
    assert chat.outbox == ["must not vanish"]
    assert not [e for e in chat.store.events() if e.type == Event.START and e.text == "must not vanish"]


def test_a_crash_mid_turn_ends_the_turn_and_stops_the_process(chat):
    chat.start()
    assert settle(chat)
    LIVE["alpha"].autoreply = False
    chat.send("start something")
    assert settle(chat, want="busy")
    runner = LIVE["alpha"]
    runner.emit(Event(type=Event.ERROR, provider="alpha", kind="crash", ok=False, error="died"))
    assert settle(chat)
    assert not runner.alive, "the dead runner must be stopped, not just dropped"
    ends = [e for e in chat.store.events() if e.type == Event.END]
    assert ends and ends[-1].extra.get("crashed"), "the turn has to be closed out"


def test_a_stopped_chat_cannot_be_restarted(chat):
    chat.start()
    assert settle(chat)
    chat.stop()
    with pytest.raises(RuntimeError):
        chat.start()


def test_a_model_change_within_one_provider_costs_no_restart(chat, two_providers):
    """Restarting would make the provider re-read the whole conversation."""
    from omni.intelligence import registry

    from omni.providers.test import dial, rung

    registry.write_cache(
        ["alpha", "beta"],
        dial(rung("alpha", "alpha-big", "high"), rung("alpha", "alpha-small", "low")),
    )
    chat.intelligence(0)
    chat.start()
    chat.send("one")
    assert settle(chat)
    runner, native = chat.runner, chat.runner.native_id

    chat.intelligence(10)
    chat.send("two")
    assert settle(chat)
    assert chat.runner is runner, "the same process should still be running"
    assert chat.runner.native_id == native
    assert chat.runner.config.model == "alpha-big"
    assert chat.runner.retuned == 1
    assert "retune" in [e.text for e in chat.store.events() if e.type == Event.CONFIG]


def test_a_provider_that_cannot_retune_is_restarted_instead(chat, two_providers):
    from omni import providers
    from omni.intelligence import registry

    from omni.providers.test import dial, rung

    registry.write_cache(
        ["alpha", "beta"],
        dial(rung("alpha", "alpha-big", "high"), rung("alpha", "alpha-small", "low")),
    )
    providers.classes("alpha")[1].tunable = False
    chat.intelligence(0)
    chat.start()
    chat.send("one")
    assert settle(chat)
    first = chat.runner

    chat.intelligence(10)
    chat.send("two")
    assert settle(chat)
    assert chat.runner is not first, "no retune means a restart"
    assert chat.runner.config.model == "alpha-big"
    assert chat.runner.resumed and chat.runner.native_id == first.native_id
    assert chat.runner.given == [], "it resumes its own session, so nothing to replay"


def test_changing_the_prompt_always_restarts(chat):
    """A system prompt is decided when a provider starts, so it cannot be retuned."""
    chat.start()
    chat.send("one")
    assert settle(chat)
    first = chat.runner
    chat.system_prompt("be terse")
    chat.send("two")
    assert settle(chat)
    assert chat.runner is not first
    assert chat.runner.config.system_prompt == "be terse"
