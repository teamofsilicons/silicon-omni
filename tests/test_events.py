"""Test group: the event vocabulary and its on-disk form."""

from omni.events import HISTORY_TYPES, Event


def test_the_wire_strings_are_pinned():
    """These end up in session files. Renaming one silently breaks reading them."""
    assert {name: getattr(Event, name) for name in
            ("START", "TEXT", "THINKING", "END", "INJECTED", "ERROR", "SWITCH_PROVIDER",
             "NEW_SESSION", "CONFIG")} == {
        "START": "start",
        "TEXT": "text",
        "THINKING": "thinking",
        "END": "end",
        "INJECTED": "injected",
        "ERROR": "error",
        "SWITCH_PROVIDER": "switch_provider",
        "NEW_SESSION": "new_session",
        "CONFIG": "config",
    }
    assert Event.TOOL.CALL == "tool.call" and Event.TOOL.RESULT == "tool.result"


def test_every_failure_gets_the_right_name():
    """The kind is what a caller branches on, so each bucket needs pinning."""
    from omni.events import AUTH, CRASH, LIMIT, UNAVAILABLE, classify

    assert classify("OAuth token is invalid") == AUTH
    assert classify("401 unauthorized") == AUTH
    assert classify("please sign in") == AUTH
    assert classify("429 rate limited") == LIMIT
    assert classify("usageLimitExceeded") == LIMIT
    assert classify("quota exhausted") == LIMIT
    assert classify("model overloaded") == UNAVAILABLE
    assert classify("responseStreamDisconnected") == UNAVAILABLE
    assert classify("503") == UNAVAILABLE
    assert classify("timeout waiting for response") == UNAVAILABLE
    assert classify("segmentation fault") == CRASH
    assert classify("") == CRASH


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
