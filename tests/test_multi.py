"""Test group: several programs, one session.

This is what the daemon bought. A session belongs to omni, not to whoever
happens to be attached: any number of clients can read it, any of them can
send, and leaving does not end it.
"""

import os
import signal
import time

from conftest import settled
from omni import Event, Inference
from omni.client import Link
from omni.client import daemon as control
from omni.providers import test as double
from omni.shared import paths


def test_two_clients_hear_the_same_turn(one, name):
    heard = []
    second = Inference.load_or_create_session(name).start()
    second.on_event(lambda event: heard.append(event.type) if event.type == Event.TEXT else None)

    one.send("hello")
    assert settled(one)
    deadline = time.time() + 5
    while not heard and time.time() < deadline:
        time.sleep(0.01)
    assert heard == [Event.TEXT], "the second client heard a turn it did not start"
    second.detach()


def test_either_client_can_send(one, name):
    second = Inference.load_or_create_session(name).start()
    one.send("from the first")
    assert settled(one)
    second.send("from the second")
    assert settled(second)
    assert double.running(name).sent == ["from the first", "from the second"]
    second.detach()


def test_a_new_client_applies_settings_queued_before_reattaching(pair):
    first, big, small = pair
    first.intelligence(0)
    first.start()
    first.send("before")
    assert settled(first)
    assert first.provider == small

    second = Inference.load_or_create_session(first.session_id, [big])
    second.intelligence(10)
    second.start()
    second.send("after")
    assert settled(second)
    assert second.provider == big
    assert second.refresh()["providers"] == [big]
    second.detach()


def test_a_client_that_arrives_late_is_told_what_it_missed(one, name):
    one.send("said before anyone else was here")
    assert settled(one)

    caught_up = []
    late = Inference.load_or_create_session(name)
    late.on_event(caught_up.append)
    late.start()
    deadline = time.time() + 5
    while not any(event.type == Event.TEXT for event in caught_up) and time.time() < deadline:
        time.sleep(0.01)
    assert [event.text for event in caught_up if event.type == Event.TEXT] == [
        "echo: said before anyone else was here"
    ]
    late.detach()


def test_a_client_can_ask_for_only_what_happens_next(one, name):
    one.send("before")
    assert settled(one)

    fresh = []
    later = Inference.load_or_create_session(name)
    later.on_event(fresh.append)
    later.start(since=-1)
    time.sleep(0.2)
    assert fresh == [], "nothing replayed"

    one.send("after")
    assert settled(one)
    deadline = time.time() + 5
    while not any(event.type == Event.TEXT for event in fresh) and time.time() < deadline:
        time.sleep(0.01)
    assert [event.text for event in fresh if event.type == Event.TEXT] == ["echo: after"]
    later.detach()


def test_detaching_leaves_the_session_running(one, name):
    one.send("hello")
    assert settled(one)
    was = double.running(name).up
    one.detach()
    time.sleep(0.2)
    assert was and double.running(name).up, "the provider stayed hot"
    assert any(
        row["snapshot"]["session"] == name for row in Inference.sessions()
    ), "and the daemon still holds the session"


def test_sending_after_a_detach_resumes_without_replaying(one):
    one.send("before")
    assert settled(one)

    heard = []
    one.on_event(lambda event: heard.append(event.text) if event.type == Event.TEXT else None)
    one.detach()
    one.send("after")
    assert settled(one)
    assert heard == ["echo: after"]


def test_stopping_is_an_instruction_and_detaching_is_not(name):
    double.install(name)
    chat = Inference.load_or_create_session(name, [name]).start()
    chat.send("hello")
    assert settled(chat)
    assert double.running(name).up
    chat.detach()
    assert double.running(name).up, "detach leaves the provider warm"
    chat.stop()
    time.sleep(0.3)
    assert not double.running(name).up, "stop means stop"
    assert not any(row["snapshot"]["session"] == name for row in Inference.sessions())


def test_an_unexpected_daemon_exit_reconnects_without_duplicate_events(one, name):
    heard = []
    one.on_event(lambda event: heard.append(event.text) if event.type == Event.TEXT else None)
    one.send("before")
    assert settled(one)

    shared = Link.shared()
    pid = Inference.daemon()["pid"]
    os.kill(pid, signal.SIGKILL)
    deadline = time.time() + 10
    while time.time() < deadline:
        if not control.listening(paths.socket()) and not one.opened and not shared.alive:
            break
        time.sleep(0.02)
    assert not one.opened and not one.finished, "a lost daemon is recoverable, not a stop"

    # Registration is process-local, so put the test provider into the new
    # daemon before this same Chat asks to reopen its on-disk session.
    double.install(name)
    one.send("after")
    assert settled(one)
    assert heard == ["echo: before", "echo: after"]
