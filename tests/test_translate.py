"""Test group: cross-provider translation — what a foreign provider gets shown."""

from omni.events import Event
from omni.translate import flatten, render, transcript


def history():
    return [
        Event(type=Event.START, text="find me kite festivals"),
        Event(type=Event.THINKING, provider="google"),
        Event(type=Event.TOOL.CALL, provider="google", tool="GoogleSearch", args={"query": "kite festivals"}, id="g1"),
        Event(type=Event.TOOL.RESULT, provider="google", tool="GoogleSearch", id="g1", result="12 results"),
        Event(type=Event.TEXT, provider="google", text="Here are three."),
        Event(type=Event.INJECTED, text="only in India"),
    ]


def test_a_single_argument_tool_reads_naturally():
    call = Event(type=Event.TOOL.CALL, tool="GoogleSearch", args={"query": "kites"})
    assert render(call) == '[GoogleSearch: "kites"]'


def test_a_multi_argument_tool_keeps_its_arguments():
    call = Event(type=Event.TOOL.CALL, tool="Edit", args={"path": "a.py", "text": "x"})
    assert render(call).startswith("[Edit: {")
    assert '"path"' in render(call)


def test_reasoning_is_never_rendered():
    assert render(Event(type=Event.THINKING, text="secret chain of thought")) == ""


def test_a_failed_tool_says_so():
    bad = Event(type=Event.TOOL.RESULT, tool="Bash", result="not found", ok=False)
    assert "failed" in render(bad)


def test_a_huge_result_reaches_the_seed_whole():
    """The spec is explicit: load the complete context, however long it is."""
    big = Event(type=Event.TOOL.RESULT, tool="Bash", result="x" * 50_000)
    assert render(big).count("x") == 50_000
    assert "…" not in render(big), "nothing may be trimmed on the way into a seed"


def test_transcript_merges_a_providers_run_into_one_turn():
    turns = transcript(history())
    assert [t["role"] for t in turns] == ["user", "assistant", "user"]
    assert "[GoogleSearch:" in turns[1]["text"]
    assert "Here are three." in turns[1]["text"]


def test_flatten_is_one_message_that_still_reads_as_a_conversation():
    text = flatten(history())
    assert text.startswith("Earlier in this conversation")
    assert "USER: find me kite festivals" in text
    assert "ASSISTANT:" in text
    assert "only in India" in text


def test_empty_history_flattens_to_nothing():
    assert flatten([]) == ""
