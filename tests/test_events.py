"""Test group: the event vocabulary and its on-disk form."""

from omni.events import HISTORY_TYPES, Event


def test_the_spec_vocabulary_is_all_there():
    for name in ("START", "TEXT", "THINKING", "END", "INJECTED", "ERROR", "SWITCH_PROVIDER", "NEW_SESSION"):
        assert isinstance(getattr(Event, name), str)
    assert Event.TOOL.CALL == "tool.call" and Event.TOOL.RESULT == "tool.result"


def test_events_round_trip_through_the_session_file():
    event = Event(type=Event.TOOL.CALL, tool="Bash", args={"command": "ls"}, id="t1", provider="claude")
    assert Event.from_dict(event.to_dict()) == event


def test_unset_fields_are_not_written():
    written = Event(type=Event.TEXT, text="hi").to_dict()
    assert set(written) == {"type", "text", "at"}


def test_a_file_written_by_a_newer_omni_still_loads():
    event = Event.from_dict({"type": "text", "text": "hi", "something_new": 1})
    assert event.text == "hi"


def test_only_conversation_events_count_as_history():
    assert Event.CONFIG not in HISTORY_TYPES
    assert Event.START in HISTORY_TYPES and Event.TOOL.RESULT in HISTORY_TYPES
