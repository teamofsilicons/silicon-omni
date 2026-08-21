"""Test group: failover — a provider that loses its login in the middle of a run.

An unauthenticated CLI cannot finish the turn it is in. By default omni takes
that provider off the chat, resolves the *same* intelligence level again over
whoever is left, and carries on there. Turn the behaviour off and the auth error
simply ends the turn.
"""

import pytest

from omni import providers
from omni.chat import Chat
from omni.events import AUTH, Event
from omni.intelligence import registry

from .fake import LIVE, dial, make, rung

TOP = 10  # the strongest rung, wherever it happens to live


@pytest.fixture
def pair():
    """alpha and beta, each with a dial of its own as well as a shared one.

    The registry serves one dial per set of providers, so dropping beta has to
    find a dial for ``["alpha"]`` — exactly what a real registry would return.
    """
    for name in ("alpha", "beta"):
        providers.register(name, *make(name))
    weak = rung("alpha", "alpha-small", "low")
    strong = rung("beta", "beta-big", "high")
    registry.write_cache(["alpha", "beta"], dial(strong, weak))
    registry.write_cache(["alpha"], dial(weak))
    registry.write_cache(["beta"], dial(strong))
    return ["alpha", "beta"]


@pytest.fixture
def chat(pair):
    chat = Chat("failover", pair)
    chat.intelligence(TOP)  # beta owns the top of the shared dial
    yield chat
    chat.stop()


def midturn_auth_failure(chat):
    """Open a turn on beta, then have beta lose its login without answering."""
    chat.start()
    assert settle(chat)
    assert chat.runner.name == "beta"
    LIVE["beta"].autoreply = False
    chat.send("still there?")
    assert settle(chat, want="busy")
    LIVE["beta"].fail(AUTH)
    assert settle(chat)


def settle(chat, want="waiting", timeout=5.0):
    import time

    end = time.time() + timeout
    while time.time() < end:
        if chat.status == want and chat.inbox.empty():
            return True
        time.sleep(0.005)
    return False


def test_the_same_level_moves_to_the_provider_that_is_left(chat):
    midturn_auth_failure(chat)
    assert chat.runner.name == "alpha", "the conversation has to carry on somewhere"
    assert chat.level == TOP, "the level is kept; only the provider serving it changed"


def test_the_unauthenticated_provider_is_taken_off_the_chat(chat):
    midturn_auth_failure(chat)
    assert chat.providers == ["alpha"]
    said = [e for e in chat.store.events() if e.text == "provider_removed"]
    assert said and said[0].provider == "beta"
    assert said[0].extra == {"why": "unauthenticated", "left": ["alpha"]}


def test_the_turn_is_closed_rather_than_left_hanging(chat):
    midturn_auth_failure(chat)
    ends = [e for e in chat.store.events() if e.type == Event.END]
    assert ends and ends[-1].extra == {"unauthenticated": "beta"}
    assert chat.status == "waiting", "a dead turn must not leave the chat busy"


def test_the_move_is_reported_as_a_switch(chat):
    midturn_auth_failure(chat)
    switches = [e for e in chat.store.events() if e.type == Event.SWITCH_PROVIDER]
    assert [(e.extra["from"], e.extra["to"]) for e in switches] == [("beta", "alpha")]


def test_the_failed_turn_is_not_replayed_on_the_new_provider(chat):
    """The message is in history, so alpha reads it — but nobody answers twice."""
    midturn_auth_failure(chat)
    assert LIVE["alpha"].sent == [], "re-driving a dead turn would double any tool it ran"
    assert "still there?" in [e.text for e in LIVE["alpha"].given]


def test_the_new_provider_is_told_everything_the_old_one_heard(chat):
    midturn_auth_failure(chat)
    assert [e.text for e in LIVE["alpha"].given] == ["still there?"]


def test_the_chat_keeps_working_after_the_failover(chat):
    midturn_auth_failure(chat)
    chat.send("hello again")
    assert settle(chat)
    assert LIVE["alpha"].sent == ["hello again"]


def test_disabling_the_behaviour_ends_the_turn_and_keeps_the_provider(chat):
    chat.disable_autoremoving_unauthenticated_providers()
    midturn_auth_failure(chat)
    assert chat.providers == ["alpha", "beta"], "nothing was dropped"
    assert chat.runner is None, "but the turn is over and the runner is down"
    errors = [e for e in chat.store.events() if e.kind == AUTH]
    assert errors, "the auth failure is still reported"
    assert not [e for e in chat.store.events() if e.type == Event.SWITCH_PROVIDER]


def test_the_last_provider_going_says_so_instead_of_hanging(pair):
    chat = Chat("failover-alone", ["beta"])
    chat.intelligence(TOP)
    try:
        midturn_auth_failure(chat)
        assert chat.providers == []
        blocked = [e for e in chat.store.events() if e.kind == "crash"]
        assert blocked and "log one back in" in blocked[-1].error
        assert chat.status == "waiting"
    finally:
        chat.stop()


def test_a_limit_error_is_not_an_auth_error(chat):
    """Quota comes back on its own. A login does not, so only auth drops anyone."""
    chat.start()
    assert settle(chat)
    LIVE["beta"].autoreply = False
    chat.send("still there?")
    assert settle(chat, want="busy")
    LIVE["beta"].fail("limit", "429 rate limited")
    assert settle(chat, want="busy")
    assert chat.providers == ["alpha", "beta"]


def test_a_second_error_from_the_provider_we_left_is_ignored(chat):
    """A dying CLI reports twice; the straggler must not take down its successor."""
    midturn_auth_failure(chat)
    assert chat.runner.name == "alpha"
    LIVE["beta"].fail(AUTH)
    assert settle(chat)
    assert chat.runner is not None and chat.runner.name == "alpha"
    assert chat.providers == ["alpha"], "beta was already gone; nothing else may follow"


def test_a_straggling_crash_does_not_kill_the_new_provider(chat):
    """Same guard, the other fatal kind — this one was wrong before the change too."""
    midturn_auth_failure(chat)
    LIVE["beta"].fail("crash", "beta exited with 1")
    assert settle(chat)
    assert chat.runner is not None and chat.runner.name == "alpha"


def test_the_dying_providers_own_end_does_not_close_its_successors_turn(chat):
    """All three adapters emit ERROR and END from the same line, so the END lands
    after the switch. Applying it there reports a turn that is still open."""
    chat.start()
    assert settle(chat)
    LIVE["beta"].autoreply = False
    chat.send("question")
    assert settle(chat, want="busy")

    # Park a second message: a settings change makes the runner stale, so flush()
    # holds it back and the failover is what finally delivers it — to alpha.
    chat.append_system_prompt("be terse")
    chat.send("parked")
    assert settle(chat, want="busy")
    assert chat.outbox == ["parked"]

    providers.EXTRA["alpha"][1].autoreply = False  # so alpha's turn stays open
    LIVE["beta"].fail(AUTH, ends=True)
    assert settle(chat, want="busy")

    assert LIVE["alpha"].sent == ["parked"], "the parked message went to the successor"
    assert chat.in_turn, "alpha is mid-turn; beta's END must not have closed it"
    assert not chat.idle, "a polling loop would otherwise stop before the answer"

    LIVE["alpha"].reply("answered")
    assert settle(chat)
    assert chat.idle
