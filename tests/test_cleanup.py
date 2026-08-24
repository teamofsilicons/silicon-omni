"""Test group: the live-test cleanup script.

Live tests run against the real ``~/.omni``, so the thing that tidies up
afterwards deletes files in a directory the user cares about. It is exercised
through its real entry point — argparse and all — and what matters is as much
what it leaves alone.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from conftest import wants_live

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "cleanup.py"
ROOT = Path(__file__).resolve().parent.parent


def run(home, *args):
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        cwd=str(ROOT),
        # A home of its own with no daemon on it: the script must not start one
        # just to ask what is open.
        env=dict(os.environ, OMNI_HOME=str(home)),
    )


@pytest.fixture
def home(tmp_path):
    """A home holding three live sessions and one the user actually wants."""
    root = tmp_path / "omni"
    sessions = root / "sessions"
    sessions.mkdir(parents=True)
    for name in ("live-claude.jsonl", "live-claude.meta.json", "keepme.jsonl"):
        (sessions / name).write_text("{}")
    (root / "cwd" / "live-switch").mkdir(parents=True)
    (root / "cwd" / "keepme").mkdir(parents=True)
    (root / "jails" / "codex").mkdir(parents=True)
    (root / "cache").mkdir(parents=True)
    (root / "cache" / "intelligence.json").write_text(
        json.dumps({"version": 2, "test": {"levels": {}}, "claude+google": {"levels": {}}})
    )
    return root


def names(home) -> set[str]:
    return {
        str(path.relative_to(home))
        for path in home.rglob("*")
        if path.is_file() or path.is_dir()
    }


def test_it_takes_the_live_leavings_out(home):
    done = run(home)
    assert done.returncode == 0, done.stderr
    left = names(home)
    assert "sessions/live-claude.jsonl" not in left
    assert "sessions/live-claude.meta.json" not in left
    assert "cwd/live-switch" not in left


def test_it_leaves_everything_else_exactly_where_it_was(home):
    before = names(home)
    run(home)
    after = names(home)
    for kept in ("sessions/keepme.jsonl", "cwd/keepme", "jails/codex"):
        assert kept in after, kept
    assert before - after == {
        "sessions/live-claude.jsonl",
        "sessions/live-claude.meta.json",
        "cwd/live-switch",
    }


def test_it_drops_the_pinned_test_dial_but_keeps_real_ones(home):
    run(home)
    blob = json.loads((home / "cache" / "intelligence.json").read_text())
    assert "test" not in blob, "the double's pinned dial is not a real one"
    assert "claude+google" in blob, "a real dial is a cache, not litter"


def test_a_dry_run_removes_nothing(home):
    before = names(home)
    done = run(home, "--dry-run")
    assert "would remove" in done.stdout
    assert names(home) == before


def test_an_empty_prefix_is_refused(home):
    done = run(home, "--prefix", "")
    assert done.returncode != 0 and "every session" in done.stderr


@pytest.mark.parametrize("prefix", ["*", "?", "live-*", "[a-z]"])
def test_a_prefix_is_a_name_not_a_pattern(home, prefix):
    """A glob would match the folders themselves. This deletes what it is given."""
    before = names(home)
    done = run(home, "--prefix", prefix)
    assert done.returncode == 0, done.stderr
    assert names(home) == before, f"{prefix!r} matched something"


@pytest.mark.parametrize("prefix", ["../", "a/b", "..", "x\\y"])
def test_a_prefix_that_looks_like_a_path_is_refused(home, prefix):
    done = run(home, "--prefix", prefix)
    assert done.returncode != 0 and "not a path" in done.stderr


def test_it_says_so_when_there_is_nothing_to_do(tmp_path):
    empty = tmp_path / "empty"
    (empty / "sessions").mkdir(parents=True)
    assert "nothing to clean" in run(empty).stdout


def test_only_an_all_live_selection_may_use_the_real_home():
    class Item:
        def __init__(self, live):
            self.live = live

        def get_closest_marker(self, name):
            return object() if name == "live" and self.live else None

    live, offline = Item(True), Item(False)
    assert wants_live([live])
    assert not wants_live([])
    assert not wants_live([offline])
    with pytest.raises(pytest.UsageError, match="cannot share one run"):
        wants_live([live, offline])
