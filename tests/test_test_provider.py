"""Test group: the shipped test provider — omni without a CLI.

``omni.providers.test`` is the provider users get for their own automatic tests,
so its promises are worth asserting: invisible until installed, identical every
run, and able to answer out of history it was only ever seeded with.
"""

import time

import pytest

from omni import Inference
from omni.chat import Chat
from omni.events import Event
from omni.providers import test as provider


def settle(chat, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        if chat.idle:
            return True
        time.sleep(0.005)
    return False


@pytest.fixture
def chat():
    provider.install()
    chat = Chat("scripted", [provider.NAME])
    yield chat
    chat.stop()


def texts(chat):
    return [e.text for e in chat.store.events() if e.type == Event.TEXT]


def test_it_is_nowhere_to_be_seen_until_it_is_installed():
    """A test double that turns up in a real run is worse than no test double."""
    only = [provider.NAME]  # asking narrowly, so no real CLI gets probed
    assert Inference.get_available_providers(only) == []
    provider.install()
    assert Inference.get_available_providers(only) == only


def test_it_covers_the_whole_dial_without_reaching_the_network(chat):
    from omni.intelligence import table

    rungs = table([provider.NAME])
    assert sorted(rungs) == list(range(11))
    assert all(r["provider"] == provider.NAME for r in rungs.values())


def test_a_turn_runs_the_full_vocabulary(chat):
    chat.start()
    chat.send("[tool:ls] please")
    assert settle(chat)
    types = [e.type for e in chat.store.events()]
    for wanted in (Event.START, Event.THINKING, Event.TOOL.CALL, Event.TOOL.RESULT, Event.TEXT, Event.END):
        assert wanted in types, wanted


def test_the_same_message_always_gets_the_same_answer(chat):
    chat.start()
    chat.send("hello")
    assert settle(chat)
    chat.send("hello")
    assert settle(chat)
    assert texts(chat) == ["echo: hello", "echo: hello"]


def test_a_tool_result_is_paired_with_its_call(chat):
    chat.start()
    chat.send("[tool:grep]")
    assert settle(chat)
    call = [e for e in chat.store.events() if e.type == Event.TOOL.CALL][0]
    result = [e for e in chat.store.events() if e.type == Event.TOOL.RESULT][0]
    assert call.id == result.id and call.tool == result.tool == "grep"


def test_it_recalls_what_it_was_told_in_this_session(chat):
    chat.start()
    chat.send("the passphrase is VIOLET-7")
    assert settle(chat)
    chat.send("[recall]")
    assert settle(chat)
    assert "VIOLET-7" in texts(chat)[-1]


def test_it_recalls_history_it_was_only_seeded_with(chat):
    """The point of the whole thing: a provider arriving late knows what happened."""
    chat.start()
    chat.send("the passphrase is VIOLET-7")
    assert settle(chat)
    chat.stop()

    again = Chat("scripted", [provider.NAME])
    again.meta.data["providers"] = {}  # as if this provider had never been here
    try:
        again.start()
        again.send("[recall]")
        assert settle(again)
        assert "VIOLET-7" in texts(again)[-1], "the seed did not carry"
    finally:
        again.stop()
