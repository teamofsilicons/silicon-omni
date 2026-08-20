"""Test group: sessions on disk — the log, the provider map, the one-owner rule."""

import json
import os

import pytest

from omni.events import Event
from omni.session import Lock, Meta, SessionBusy, Store
from omni.shared import paths


def test_the_session_file_is_the_event_log():
    store = Store("s")
    store.append(Event(type=Event.START, text="hi"))
    store.append(Event(type=Event.TEXT, text="hello"))
    lines = paths.session_file("s").read_text().strip().split("\n")
    assert len(lines) == 2
    assert json.loads(lines[0])["type"] == "start"
    assert json.loads(lines[0])["session"] == "s"


def test_sequence_numbers_continue_across_reopens():
    first = Store("s")
    first.append(Event(type=Event.START, text="a"))
    second = Store("s")
    second.append(Event(type=Event.TEXT, text="b"))
    assert [e.seq for e in second.events()] == [0, 1]


def test_history_leaves_out_bookkeeping():
    store = Store("s")
    store.append(Event(type=Event.START, text="a"))
    store.append(Event(type=Event.CONFIG, text="intelligence"))
    store.append(Event(type=Event.NEW_SESSION))
    store.append(Event(type=Event.TEXT, text="b"))
    assert [e.text for e in store.history()] == ["a", "b"]


def test_history_can_start_part_way_through():
    store = Store("s")
    for text in "abcd":
        store.append(Event(type=Event.TEXT, text=text))
    assert [e.text for e in store.history(since=2)] == ["c", "d"]


def test_a_half_written_line_is_skipped_not_fatal():
    store = Store("s")
    store.append(Event(type=Event.TEXT, text="good"))
    with open(paths.session_file("s"), "a") as fh:
        fh.write('{"type": "text", "te')
    assert [e.text for e in Store("s").history()] == ["good"]


def test_meta_remembers_each_providers_own_session():
    meta = Meta("s")
    meta.bind("claude", "uuid-b")
    meta.mark_synced("claude", 4)
    meta.bind("google", "conv-c")
    again = Meta("s")
    assert again.native("claude") == {"id": "uuid-b", "synced": 4}
    assert again.native("google")["id"] == "conv-c"
    assert again.native("openai") == {"id": "", "synced": -1}


def test_one_live_chat_per_session_id():
    held = Lock("s").acquire()
    with pytest.raises(SessionBusy):
        Lock("s").acquire()
    held.release()
    Lock("s").acquire().release()


def test_a_session_whose_owner_died_is_reclaimed():
    paths.ensure(paths.sessions())
    paths.lock_file("s").write_text(json.dumps({"pid": 4_000_000, "at": "then"}))
    assert Lock("s").acquire().held


def test_a_lock_is_only_released_by_its_owner():
    held = Lock("s").acquire()
    paths.lock_file("s").write_text(json.dumps({"pid": os.getpid() + 1, "at": "then"}))
    held.release()
    assert paths.lock_file("s").exists(), "someone else's lock must survive"
