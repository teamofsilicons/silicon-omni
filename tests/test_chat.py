"""The public API, end to end.

Group: everything a program written against omni actually touches — turns,
ordering, settings, switching and failover — driven through a real daemon over a
real socket.
"""

import threading
from pathlib import Path

import pytest
from conftest import in_turn, settled

import omni.inference as inference_module
from omni import Event, Inference, NoAnswer
from omni.chat import Chat
from omni.client import DaemonError, Link
from omni.providers import test as double

# ------------------------------------------------------------- vocabulary

def test_python_sends_the_ask_in_every_shape(
    monkeypatch, name
):
    class StubLink:
        alive = True

        def __init__(self):
            self.requests = []
            self.listener = None

        def listen(self, session, handler):
            self.listener = handler

        def unlisten(self, session):
            self.listener = None

        def call(self, op, **fields):
            self.requests.append((op, fields))
            if op == "open":
                return {
                    "snapshot": {
                        "session": name,
                        "status": "waiting",
                        "ask": {"how": "intelligence", "value": 7},
                        "effort": "high",
                        "seq": -1,
                        "queued": 0,
                        "in_turn": False,
                    }
                }
            if op == "status":
                return {
                    "snapshot": {
                        "session": name,
                        "status": "waiting",
                        "ask": {"how": "key", "key": "code"},
                        "effort": "medium",
                        "seq": -1,
                        "queued": 0,
                        "in_turn": False,
                    }
                }
            return {}

        def close(self):
            self.alive = False

    link = StubLink()
    monkeypatch.setattr(Link, "open", classmethod(lambda cls: link))

    chat = Chat(name, [])
    chat.model(intelligence=7)
    chat.start()
    assert link.requests[0][0] == "open"
    assert link.requests[0][1]["value"] == [
        {"what": "model", "value": {"how": "intelligence", "value": 7}}
    ]
    assert chat.current_ask == {"how": "intelligence", "value": 7}
    assert chat.effort == "high"
    assert "level" not in chat.state

    chat.model(intelligence=8, bench="terminal-bench")
    assert link.requests[-1] == (
        "set",
        {
            "session": name,
            "what": "model",
            "value": {"how": "intelligence", "value": 8, "bench": "terminal-bench"},
        },
    )

    chat.model("code")
    assert link.requests[-1][1]["value"] == {"how": "key", "key": "code"}

    chat.model(model="gemini-3.7-flash-low", provider="google", effort="", fast=True)
    assert link.requests[-1][1]["value"] == {
        "how": "model",
        "model": "gemini-3.7-flash-low",
        "effort": "",
        "fast": True,
        "provider": "google",
    }

    for bad in ({}, {"key": "code", "intelligence": 4}):
        try:
            chat.model(**bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f"saying {bad} should be refused")

    refreshed = chat.refresh()
    assert refreshed["ask"] == {"how": "key", "key": "code"}
    assert refreshed["effort"] == "medium"
    assert "level" not in refreshed
    chat.detach()





def test_unauthenticated_provider_autoremoval_has_both_toggles(name):
    chat = Chat(name, [])
    chat.disable_autoremoving_unauthenticated_providers()
    chat.enable_autoremoving_unauthenticated_providers()
    assert chat.pending == [("autoremove", False), ("autoremove", True)]


def test_a_chat_context_detaches_instead_of_stopping(name):
    calls = []
    chat = Chat(name, [])
    chat.start = lambda since=0: calls.append("start") or chat
    chat.detach = lambda: calls.append("detach")
    chat.stop = lambda: calls.append("stop")

    with chat as attached:
        assert attached is chat

    assert calls == ["start", "detach"]


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

def test_the_opening_python_process_cwd_pins_a_new_session(
    name, tmp_path, monkeypatch
):
    double.install(name)
    first_dir = tmp_path / "first-client"
    later_dir = tmp_path / "later-client"
    first_dir.mkdir()
    later_dir.mkdir()

    monkeypatch.chdir(first_dir)
    first = Inference.load_or_create_session(name, [name]).start()
    assert Path(first.refresh()["cwd"]).resolve() == first_dir.resolve()
    first.stop()

    monkeypatch.chdir(later_dir)
    restored = Inference.load_or_create_session(name).start()
    assert Path(restored.refresh()["cwd"]).resolve() == first_dir.resolve()
    restored.stop()


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
    chat.model(intelligence=0)
    chat.start()
    chat.send("remember VIOLET-7")
    assert settled(chat)
    assert chat.provider == small

    chat.model(intelligence=10)
    chat.send("[recall]")
    assert settled(chat)
    assert chat.provider == big
    recalled = [event.text for event in chat.history() if event.type == Event.TEXT][-1]
    assert "VIOLET-7" in recalled, recalled


def test_nothing_changes_mid_turn(pair):
    chat, big, small = pair
    chat.model(intelligence=0)
    chat.start()
    chat.send("first")
    assert settled(chat)

    double.running(small).autoreply = False
    chat.send("second")
    assert in_turn(chat)
    chat.model(intelligence=10)
    import time

    time.sleep(0.3)
    assert chat.provider == small, "still where the turn started"

    double.running(small).reply("done")
    assert settled(chat)
    assert chat.provider == big, "applied at the boundary"


def test_settings_asked_for_before_start_are_applied_before_anything_runs(pair):
    chat, big, small = pair
    chat.model(intelligence=10)
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
    chat.model(intelligence=10)
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
    chat.model(intelligence=10)
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


def test_asking_for_providers_omni_cannot_answer_for_is_reported(one):
    one.active_inference_providers(["nobody-at-all"])
    one.send("hello")
    assert settled(one)
    trouble = [
        event for event in one.history() if event.type == Event.ERROR and "no answer" in event.error
    ]
    assert trouble, [event.type for event in one.history()]
    assert one.status == "waiting", "reported, not stuck on busy"


def test_having_nothing_to_answer_with_preserves_the_public_exception(name):
    with pytest.raises(NoAnswer, match="no answer"):
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


def test_lifecycle_calls_do_not_restart_a_dead_daemon(monkeypatch, name):
    """Leaving a disconnected session is local cleanup, not a daemon launch."""

    class DeadLink:
        alive = False

        def unlisten(self, session):
            pass

        def close(self):
            pass

    def unexpected_open(cls):
        raise AssertionError("detach/stop must not start a daemon")

    monkeypatch.setattr(Link, "open", classmethod(unexpected_open))

    detached = Chat(name, [name])
    detached.connection = DeadLink()
    detached.opened = True
    detached._started = True
    detached.detach()

    stopped = Chat(f"{name}-stop", [name])
    stopped.connection = DeadLink()
    stopped.opened = True
    stopped._started = True
    with pytest.raises(DaemonError, match="connection is gone"):
        stopped.stop()
    assert not stopped.finished and not stopped.opened
    assert stopped.connection is None


@pytest.mark.parametrize(
    ("operation", "failure"),
    [
        ("stop", "stop was refused"),
        ("stop", "stop got no answer in 1s"),
        ("detach", "detach was refused"),
        ("detach", "detach got no answer in 1s"),
    ],
)
def test_lifecycle_rpc_failures_are_surfaced_and_reloadable(
    monkeypatch, name, operation, failure
):
    class StubLink:
        def __init__(self, failing=None):
            self.alive = True
            self.failing = failing
            self.listener = None
            self.ops = []
            self.unlistened = []
            self.closed = False

        def listen(self, session, handler):
            self.listener = handler

        def unlisten(self, session):
            self.unlistened.append(session)
            self.listener = None

        def call(self, op, **fields):
            self.ops.append(op)
            if op == operation and self.failing:
                raise DaemonError(self.failing)
            if op == "open":
                return {
                    "snapshot": {
                        "session": name,
                        "status": "waiting",
                        "seq": -1,
                        "queued": 0,
                        "in_turn": False,
                    }
                }
            return {}

        def close(self):
            self.closed = True
            self.alive = False

    failed = StubLink(failing=failure)
    chat = Chat(name, [name])
    chat.connection = failed
    chat.opened = True
    chat._started = True
    chat.state["status"] = "waiting"

    with pytest.raises(DaemonError, match="refused|no answer"):
        getattr(chat, operation)()

    assert failed.ops == [operation]
    assert failed.unlistened == [name] and failed.closed
    assert chat.connection is None
    assert not chat.opened and not chat.finished
    assert chat.status == "waiting"

    recovered = StubLink()
    monkeypatch.setattr(Link, "open", classmethod(lambda cls: recovered))
    assert chat.start(since=0) is chat
    assert chat.opened and not chat.finished

    getattr(chat, operation)()
    calls = recovered.ops.count(operation)
    getattr(chat, operation)()  # success is idempotent
    assert recovered.ops.count(operation) == calls == 1
    if operation == "stop":
        assert chat.finished and chat.status == "stopped"
    else:
        assert not chat.opened and not chat.finished
        chat.stop()


@pytest.mark.parametrize("operation", ["stop", "detach"])
def test_lifecycle_state_changes_only_after_daemon_acknowledgement(name, operation):
    entered = threading.Event()
    release = threading.Event()

    class BlockingLink:
        alive = True

        def unlisten(self, session):
            pass

        def call(self, op, **fields):
            if op == operation:
                entered.set()
                release.wait(timeout=5)
            return {}

        def close(self):
            self.alive = False

    chat = Chat(name, [name])
    chat.connection = BlockingLink()
    chat.opened = True
    chat._started = True
    finished = threading.Event()

    def lifecycle_call():
        getattr(chat, operation)()
        finished.set()

    calling = threading.Thread(target=lifecycle_call)
    calling.start()
    assert entered.wait(timeout=1)
    assert chat.opened and not chat.finished
    assert not finished.is_set()

    release.set()
    assert finished.wait(timeout=1)
    calling.join(timeout=1)
    assert not chat.opened
    assert chat.finished is (operation == "stop")
    if operation == "detach":
        chat.stop()


def test_a_stale_disconnect_cannot_close_or_strand_a_reopened_chat(monkeypatch, name):
    """A slow old callback may overlap reopen without owning its new link."""

    def snapshot(seq):
        return {
            "session": name,
            "status": "waiting",
            "seq": seq,
            "queued": 0,
            "in_turn": False,
        }

    def text(seq, said):
        return {
            "stream": "event",
            "session": name,
            "event": {"type": Event.TEXT, "seq": seq, "text": said},
            "snapshot": snapshot(seq),
        }

    class StubLink:
        def __init__(self, replay=None, seq=-1):
            self.alive = True
            self.replay = replay
            self.snapshot = snapshot(seq)
            self.listener = None

        def listen(self, session, handler):
            self.listener = handler

        def unlisten(self, session):
            self.listener = None

        def call(self, op, **fields):
            if op == "open":
                if self.replay is not None:
                    self.listener(self.replay)
                return {"snapshot": self.snapshot}
            return {}

        def close(self):
            self.alive = False

        def emit(self, frame):
            self.listener(frame)

    old = StubLink()
    new = StubLink(replay=text(1, "new"), seq=1)
    links = iter((old, new))
    monkeypatch.setattr(Link, "open", classmethod(lambda cls: next(links)))

    chat = Chat(name, [name]).start()
    entered = threading.Event()
    release = threading.Event()
    delivered = threading.Event()
    callbacks = []

    @chat.on_event
    def observe(event):
        callbacks.append(event.seq)
        if event.seq == 0:
            entered.set()
            release.wait(timeout=5)
        elif event.seq == 1:
            delivered.set()

    old.emit(text(0, "old"))
    assert entered.wait(timeout=1)
    old.alive = False
    old.emit({"stream": "disconnected", "session": name})

    reopened = threading.Event()
    errors = []

    def reopen():
        try:
            chat.start(since=1)
        except BaseException as exc:
            errors.append(exc)
        finally:
            reopened.set()

    opening = threading.Thread(target=reopen)
    opening.start()
    assert reopened.wait(timeout=1), "reopen waited for the old user callback"
    opening.join(timeout=1)
    assert not errors
    assert chat.opened
    assert chat.seen == 0, "the snapshot must not advance the callback cursor"

    release.set()
    assert delivered.wait(timeout=1)
    assert callbacks == [0, 1]
    assert chat.opened and chat.seen == 1
    chat.detach()
