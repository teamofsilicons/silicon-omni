"""The public API, end to end.

Group: everything a program written against omni actually touches — turns,
ordering, settings, switching and failover — driven through a real daemon over a
real socket.
"""

import pytest

from conftest import in_turn, settled
from omni import Event, Inference, NoDial
from omni.chat import Chat
from omni.client import DaemonError, Link
from omni.providers import test as double


# ---------------------------------------------------------------- one turn

def test_a_message_gets_an_answer(one):
    one.send("hello")
    assert settled(one)
    said = [event.text for event in one.history() if event.type == Event.TEXT]
    assert said == ["echo: hello"]


def test_the_status_reads_busy_before_send_returns(one, name):
    double.running(name).autoreply = False
    one.send("hello")
    assert one.status == "busy", "a polling loop must never see a false lull"
    double.running(name).reply("done")
    assert settled(one)
    assert one.status == "waiting"


def test_callbacks_fire_in_the_order_things_happened(one):
    seen = []
    one.on_event(lambda event: seen.append(event.type))
    one.send("[tool:ls] please")
    assert settled(one)
    assert seen.index(Event.START) < seen.index(Event.TOOL.CALL)
    assert seen.index(Event.TOOL.CALL) < seen.index(Event.TOOL.RESULT)
    assert seen.index(Event.TOOL.RESULT) < seen.index(Event.END)


def test_a_broken_handler_does_not_take_the_session_down(one):
    logged = []

    @one.on_event
    def explode(event):
        raise ValueError("nope")

    @one.logs
    def note(event):
        logged.append(event)

    one.send("hello")
    assert settled(one)
    assert any(event.type == Event.TEXT for event in logged), "the run carried on"
    blamed = [event for event in logged if event.type == Event.ERROR and event.kind == "handler"]
    assert blamed, "and the broken handler was named"
    assert "explode" in blamed[0].error


def test_a_message_sent_mid_turn_lands_inside_it(one, name):
    double.running(name).autoreply = False
    one.send("first")
    assert in_turn(one)
    one.send("second")
    import time

    time.sleep(0.3)
    kinds = [event.type for event in one.history()]
    assert kinds.count(Event.INJECTED) == 1
    assert kinds.count(Event.START) == 1
    double.running(name).reply("done")
    assert settled(one)


# ------------------------------------------------------------- persistence

def test_the_session_outlives_the_object_that_opened_it(one, name):
    one.send("remember VIOLET-7")
    assert settled(one)
    one.detach()

    again = Inference.load_or_create_session(name).start()
    said = [event.text for event in again.history() if event.type == Event.TEXT]
    assert said == ["echo: remember VIOLET-7"]
    assert again.refresh()["provider"] == name, "and its provider is still up"
    again.detach()


def test_seq_only_ever_goes_up(one):
    one.send("one")
    assert settled(one)
    one.send("two")
    assert settled(one)
    seqs = [event.seq for event in one.history()]
    assert seqs == sorted(set(seqs)) == seqs


# ---------------------------------------------------------------- switching

def test_raising_intelligence_moves_the_conversation_and_keeps_it(pair):
    chat, big, small = pair
    chat.intelligence(0)
    chat.start()
    chat.send("remember VIOLET-7")
    assert settled(chat)
    assert chat.provider == small

    chat.intelligence(10)
    chat.send("[recall]")
    assert settled(chat)
    assert chat.provider == big
    recalled = [event.text for event in chat.history() if event.type == Event.TEXT][-1]
    assert "VIOLET-7" in recalled, recalled


def test_nothing_changes_mid_turn(pair):
    chat, big, small = pair
    chat.intelligence(0)
    chat.start()
    chat.send("first")
    assert settled(chat)

    double.running(small).autoreply = False
    chat.send("second")
    assert in_turn(chat)
    chat.intelligence(10)
    import time

    time.sleep(0.3)
    assert chat.provider == small, "still where the turn started"

    double.running(small).reply("done")
    assert settled(chat)
    assert chat.provider == big, "applied at the boundary"


def test_settings_asked_for_before_start_are_applied_before_anything_runs(pair):
    chat, big, small = pair
    chat.intelligence(10)
    chat.start()
    chat.send("hello")
    assert settled(chat)
    launches = [
        event for event in chat.history() if event.type == Event.CONFIG and event.text == "launch"
    ]
    assert len(launches) == 1, "the provider that came up is the one that was asked for"
    assert launches[0].provider == big


# ----------------------------------------------------------------- failover

def test_losing_a_login_moves_the_chat_to_whoever_is_left(pair):
    chat, big, small = pair
    chat.intelligence(10)
    chat.start()
    chat.send("hello")
    assert settled(chat)

    double.running(big).autoreply = False
    chat.send("again")
    assert in_turn(chat)
    double.running(big).fail("auth", ends=True)
    assert settled(chat)

    assert chat.refresh()["providers"] == [small]
    removed = [
        event
        for event in chat.history()
        if event.type == Event.CONFIG and event.text == "provider_removed"
    ]
    assert len(removed) == 1 and removed[0].provider == big

    chat.send("[recall]")
    assert settled(chat)
    recalled = [event.text for event in chat.history() if event.type == Event.TEXT][-1]
    assert "hello" in recalled, recalled


def test_turning_the_failover_off_reports_and_stops(pair):
    chat, big, small = pair
    chat.disable_autoremoving_unauthenticated_providers()
    chat.intelligence(10)
    chat.start()
    chat.send("hello")
    assert settled(chat)
    double.running(big).fail("auth", ends=True)
    assert settled(chat)
    assert sorted(chat.refresh()["providers"]) == sorted([big, small])


# ------------------------------------------------------------------ refusals

def test_a_stopped_session_says_so_rather_than_hanging(one):
    one.stop()
    with pytest.raises(RuntimeError, match="stopped"):
        one.send("hello")


def test_asking_for_providers_omni_has_no_dial_for_is_reported(one):
    one.active_inference_providers(["nobody-at-all"])
    one.send("hello")
    assert settled(one)
    trouble = [
        event for event in one.history() if event.type == Event.ERROR and "no dial" in event.error
    ]
    assert trouble, [event.type for event in one.history()]
    assert one.status == "waiting", "reported, not stuck on busy"


def test_asking_for_a_missing_dial_preserves_the_public_exception(name):
    with pytest.raises(NoDial, match="no dial"):
        Inference.dial([f"nobody-{name}"])


def test_a_failed_start_can_be_detached_retried_and_stopped(monkeypatch, name):
    """A refused open leaves no listener or callback thread behind."""

    class StubLink:
        def __init__(self, fail=False):
            self.alive = True
            self.fail = fail
            self.listeners = {}
            self.ops = []

        def listen(self, session, handler):
            self.listeners[session] = handler

        def unlisten(self, session):
            self.listeners.pop(session, None)

        def call(self, op, **fields):
            self.ops.append(op)
            if self.fail and op == "open":
                raise DaemonError("open was refused")
            if op == "open":
                return {
                    "snapshot": {
                        "session": name,
                        "status": "waiting",
                        "seq": -1,
                        "in_turn": False,
                    }
                }
            return {}

        def close(self):
            self.alive = False

    failed, recovered = StubLink(fail=True), StubLink()
    links = iter((failed, recovered))
    monkeypatch.setattr(Link, "open", classmethod(lambda cls: next(links)))

    chat = Chat(name, [name])
    with pytest.raises(DaemonError, match="refused"):
        chat.start()
    assert chat.connection is None
    assert chat.caller is None
    assert failed.listeners == {} and not failed.alive

    # Neither operation should open a daemon just to leave a start that failed.
    chat.detach()
    assert chat.start() is chat
    chat.detach()
    chat.stop()
    assert recovered.ops == ["open", "detach", "stop"]
