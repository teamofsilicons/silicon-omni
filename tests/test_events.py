"""Test group: the event vocabulary, and that Python agrees with the daemon.

The strings end up in session files and in every callback, and they are now
declared twice — once in Rust, once here. Anything that could drift is pinned.
"""

from omni import Event
from omni.events import ALWAYS, AUTH, CRASH, LIMIT, UNAVAILABLE


def test_the_wire_strings_are_pinned():
    """Renaming one silently breaks reading every session file ever written."""
    assert {
        name: getattr(Event, name)
        for name in (
            "START", "TEXT", "THINKING", "END", "INJECTED", "ERROR",
            "SWITCH_PROVIDER", "NEW_SESSION", "CONFIG",
        )
    } == {
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
    assert not hasattr(Event, "SEED"), "provider-internal seed records are not public events"


def test_the_failure_kinds_are_pinned():
    """What a caller branches on when a turn goes wrong."""
    assert (AUTH, LIMIT, UNAVAILABLE, CRASH) == ("auth", "limit", "unavailable", "crash")


def test_events_round_trip_through_the_session_file():
    event = Event(
        type=Event.TOOL.CALL, tool="Bash", args={"command": "ls"}, id="t1", provider="claude-code-cli"
    )
    assert Event.from_dict(event.to_dict()) == event


def test_unset_fields_are_not_written():
    assert set(Event(type=Event.TEXT, text="hi").to_dict()) == {"v", "type", "text", "at"}
    assert ALWAYS == ("v", "type", "at")


def test_a_file_written_by_a_newer_omni_still_loads():
    assert Event.from_dict({"type": "text", "text": "hi", "something_new": 1}).text == "hi"


def test_a_pre_versioned_file_defaults_to_schema_one():
    assert Event.from_dict({"type": "text", "text": "old"}).v == 1



def test_the_daemon_and_python_describe_an_event_the_same_way(one):
    """The one that matters: two languages, one schema.

    Everything above pins this side. This pins that the other side agrees —
    which is the only way a rename in Rust gets caught here.
    """
    from conftest import settled

    one.send("[tool:ls] go")
    assert settled(one)
    by_type = {event.type: event for event in one.history()}
    assert {Event.START, Event.TEXT, Event.TOOL.CALL, Event.TOOL.RESULT, Event.END} <= set(by_type)

    call = by_type[Event.TOOL.CALL]
    assert call.tool == "ls" and call.args == {"input": "ls"} and call.id
    assert by_type[Event.TOOL.RESULT].ok is True
    assert all(event.at.endswith("Z") for event in one.history()), "one time format, everywhere"
    assert by_type[Event.TEXT].seq >= 0 and by_type[Event.TEXT].session
