"""Test group: the shipped test provider.

It is part of the package, not part of the test suite — anyone testing their own
code against omni gets the same double omni tests itself with. So the knobs it
offers are worth their own tests.
"""

import pytest

from conftest import in_turn, settled
from omni import Event, Inference
from omni.client import DaemonError
from omni.providers import test as double


def test_it_echoes_what_you_send(one):
    one.send("hello")
    assert settled(one)
    assert [event.text for event in one.history() if event.type == Event.TEXT] == ["echo: hello"]


def test_a_tool_marker_runs_a_tool(one, name):
    one.send("[tool:ls] and [tool:cat]")
    assert settled(one)
    calls = [event.tool for event in one.history() if event.type == Event.TOOL.CALL]
    results = [event.tool for event in one.history() if event.type == Event.TOOL.RESULT]
    assert calls == ["ls", "cat"] == results


def test_recall_answers_out_of_history_it_was_only_seeded_with(pair):
    chat, big, small = pair
    chat.intelligence(0)
    chat.start()
    chat.send("the passphrase is VIOLET-7")
    assert settled(chat)
    chat.intelligence(10)
    chat.send("[recall]")
    assert settled(chat)
    assert "VIOLET-7" in [e.text for e in chat.history() if e.type == Event.TEXT][-1]
    assert double.running(big).resumed is False, "it was seeded, not resumed"


def test_autoreply_off_holds_the_turn_open(one, name):
    double.running(name).autoreply = False
    one.send("hello")
    assert in_turn(one)
    assert not one.idle
    double.running(name).reply("at last")
    assert settled(one)
    assert [e.text for e in one.history() if e.type == Event.TEXT] == ["at last"]


def test_it_can_be_made_to_fail_the_way_a_real_cli_does(one, name):
    one.send("hello")
    assert settled(one)
    double.running(name).fail("crash", "the pipe went away", ends=True)
    assert settled(one)
    broken = [e for e in one.history() if e.type == Event.ERROR]
    assert broken and broken[-1].error == "the pipe went away"


def test_the_knobs_read_back(one, name):
    live = double.running(name)
    assert live.autoreply is True and live.tunable is True and live.defer is False
    live.defer = True
    assert live.defer is True
    assert live.tunable is True, "one knob at a time"


def test_what_it_was_sent_is_recorded(one, name):
    one.send("one")
    assert settled(one)
    one.send("two")
    assert settled(one)
    assert double.running(name).sent == ["one", "two"]
    assert double.running(name).up is True


def test_nothing_is_registered_until_you_ask(name):
    assert name not in Inference.get_available_providers()
    double.install(name)
    assert name in Inference.get_available_providers()
    assert name in double.installed()


def test_asking_about_a_provider_that_is_not_running_says_so(name):
    double.install(name)
    with pytest.raises(DaemonError, match="no test provider"):
        double.running(name)
