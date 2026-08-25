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
from omni.events import LIMIT, Event
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


def talk(session_id, providers, intelligence=CHEAPEST):
    """A live chat in the real omni home, working in a directory of its own.

    The directory is stable across runs on purpose: claude resumes by working
    directory, so a fresh temp dir every time would only ever test reseeding.

    Sessions are stable too — that is the point of them — so the file may
    already hold everything a previous run did. ``chat.from_here`` is the seq
    it stood at before this run, and every assertion below is scoped to what
    came after it. Asserting over the whole file passes exactly once, on a
    clean machine, and has told you nothing since.
    """
    chat = Inference.load_or_create_session(session_id, providers)
    chat.from_here = max((event.seq for event in chat.history()), default=-1) + 1
    chat.cwd(str(paths.ensure(paths.home() / "cwd" / session_id)))
    chat.intelligence(intelligence)
    chat.disable_subagents()
    chat.disable_mcp()
    return chat


def this_run(chat):
    """Only the events this run appended, whatever the session already held."""
    return chat.history(since=chat.from_here)


def said(chat):
    return " ".join(e.text for e in this_run(chat) if e.type == Event.TEXT)


def has_quota(provider):
    """Known-full windows cannot run an inference test, even when auth is healthy."""
    limits = getattr(Inference, provider).limits
    if not isinstance(limits, dict):
        return False
    return not any(
        window.get("used") is not None and window["used"] >= 1.0
        for window in limits.values()
    )


def skip_if_limited(chat, provider):
    """A window can fill between the free account probe and the real turn."""
    failures = [
        event
        for event in this_run(chat)
        if event.type == Event.ERROR and event.kind == LIMIT and event.provider == provider
    ]
    if failures:
        pytest.skip(f"{provider} has no live inference quota: {failures[-1].error}")


@pytest.fixture(params=["claude", "openai", "google"])
def provider(request):
    if request.param not in Inference.get_available_providers():
        pytest.skip(f"{request.param} is not installed or not logged in")
    return request.param


def test_a_provider_answers_runs_a_tool_and_remembers(provider):
    if not has_quota(provider):
        pytest.skip(f"{provider} has no live inference quota")
    chat = talk(f"live-{provider}", [provider])
    try:
        chat.start()
        chat.send("reply with exactly: OK")
        assert settle(chat)
        skip_if_limited(chat, provider)
        chat.send("Run the shell command: echo omni-live. Then reply with exactly: DONE")
        assert settle(chat)
        skip_if_limited(chat, provider)
        chat.send("what was the exact output of that command? one word.")
        assert settle(chat)
        skip_if_limited(chat, provider)
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
    available = Inference.get_available_providers()
    have = [p for p in ("google", "claude", "openai") if p in available and has_quota(p)]
    if len(have) < 2:
        pytest.skip("needs two providers with live inference quota to switch between")
    first, second = have[0], have[1]
    chat = talk("live-switch", [first])

    def to(name):
        # A remote dial is allowed to rank one member of a provider pair at
        # every level. Narrowing the active set still crosses the exact same
        # translation/resume boundary without making this test depend on the
        # ranking deployed today.
        chat.active_inference_providers([name])
        chat.intelligence(CHEAPEST)

    try:
        chat.start()
        to(first)
        chat.send("Remember: the passphrase is VIOLET-7. Reply with exactly: STORED")
        assert settle(chat)
        skip_if_limited(chat, first)
        to(second)
        chat.send("What is the passphrase? Answer with just the passphrase.")
        assert settle(chat)
        skip_if_limited(chat, second)
        assert "VIOLET-7" in said(chat).upper(), "history did not survive the move"
        to(first)
        chat.send("Say the passphrase once more, just the value.")
        assert settle(chat)
        skip_if_limited(chat, first)
    finally:
        chat.stop()

    kinds = [e.type for e in this_run(chat)]
    assert kinds.count(Event.SWITCH_PROVIDER) >= 2
