"""Test group: live providers.

These drive the real CLIs, so they need you to be logged in and they spend a
little quota. They are excluded by default::

    pytest              # everything else
    pytest -m live      # these

Each one picks the cheapest rung the provider has.
"""

import time

import pytest

from omni import Inference
from omni.chat import Chat
from omni.events import Event
from omni.session import Store
from omni.shared import paths

pytestmark = pytest.mark.live

CHEAPEST = 0  # the bottom of every dial, whoever you are signed into


def settle(chat, timeout=400):
    end = time.time() + timeout
    while time.time() < end:
        if chat.idle:
            return True
        time.sleep(0.1)
    return False


def talk(session_id, providers, level=CHEAPEST):
    """A live chat in the real omni home, working in a directory of its own.

    The directory is stable across runs on purpose: claude resumes by working
    directory, so a fresh temp dir every time would only ever test reseeding.

    Sessions are stable too — that is the point of them — so the file may
    already hold everything a previous run did. ``chat.from_here`` is the seq
    it stood at before this run, and every assertion below is scoped to what
    came after it. Asserting over the whole file passes exactly once, on a
    clean machine, and has told you nothing since.
    """
    chat = Chat(session_id, providers)
    chat.from_here = chat.store.seq + 1
    chat.cwd(str(paths.ensure(paths.home() / "cwd" / session_id)))
    chat.intelligence(level)
    chat.disable_subagents()
    chat.disable_mcp()
    return chat


def this_run(chat):
    """Only the events this run appended, whatever the session already held."""
    return Store(chat.session_id).events(since=chat.from_here)


def said(chat):
    return " ".join(e.text for e in this_run(chat) if e.type == Event.TEXT)


@pytest.fixture(params=["claude", "openai", "google"])
def provider(request):
    if request.param not in Inference.get_available_providers():
        pytest.skip(f"{request.param} is not installed or not logged in")
    return request.param


def test_a_provider_answers_runs_a_tool_and_remembers(provider):
    chat = talk(f"live-{provider}", [provider])
    try:
        chat.start()
        chat.send("reply with exactly: OK")
        assert settle(chat)
        chat.send("Run the shell command: echo omni-live. Then reply with exactly: DONE")
        assert settle(chat)
        chat.send("what was the exact output of that command? one word.")
        assert settle(chat)
    finally:
        chat.stop()

    kinds = [e.type for e in this_run(chat)]
    assert kinds.count(Event.END) == 3, "three turns, three endings"
    assert Event.TOOL.CALL in kinds and Event.TOOL.RESULT in kinds
    assert "omni-live" in said(chat), "the third turn had to remember the first two"


def test_auth_and_limits_answer_without_a_turn(provider):
    account = getattr(Inference, provider)
    assert account.auth_status == "authenticated"
    limits = account.limits
    assert set(limits) == {"5h", "7d"}
    assert all(set(w) >= {"used", "reset"} for w in limits.values())


def test_a_conversation_survives_moving_between_providers():
    have = [p for p in ("google", "claude", "openai") if p in Inference.get_available_providers()]
    if len(have) < 2:
        pytest.skip("needs two providers to switch between")
    first, second = have[0], have[1]
    chat = talk("live-switch", [first, second])

    def to(name):
        for level in range(11):
            if chat.rung()["provider"] == name:
                return
            chat.intelligence(level)
        pytest.skip(f"no rung resolves to {name}")

    try:
        chat.start()
        to(first)
        chat.send("Remember: the passphrase is VIOLET-7. Reply with exactly: STORED")
        assert settle(chat)
        to(second)
        chat.send("What is the passphrase? Answer with just the passphrase.")
        assert settle(chat)
        assert "VIOLET-7" in said(chat).upper(), "history did not survive the move"
        to(first)
        chat.send("Say the passphrase once more, just the value.")
        assert settle(chat)
    finally:
        chat.stop()

    kinds = [e.type for e in this_run(chat)]
    assert kinds.count(Event.SWITCH_PROVIDER) >= 2
