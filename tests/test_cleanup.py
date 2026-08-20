"""Test group: the live-test cleanup script.

Live tests run against the real ``~/.omni``, so the thing that tidies up afterwards
deletes files in a directory the user cares about. It is exercised through its real
entry point — argparse and all — and what matters is as much what it leaves alone.
"""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from omni.providers import test as provider
from omni.shared import paths

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "cleanup.py"


def run(home, *args):
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=dict(os.environ, OMNI_HOME=str(home)),
    )


@pytest.fixture
def home(tmp_path):
    """A home holding three live sessions and one the user actually wants.

    ``omni_home`` in conftest already points OMNI_HOME here.
    """
    root = tmp_path / "omni"
    paths.ensure(paths.sessions())
    for name in ("live-claude.jsonl", "live-claude.meta.json", "live-claude.lock", "keepme.jsonl"):
        (paths.sessions() / name).write_text("{}")
    paths.ensure(root / "jails" / "live-openai" / "codex")
    paths.ensure(root / "cwd" / "live-switch")
    paths.ensure(root / "jails" / "keepme")
    provider.install()  # pins its dial into this home's cache
    return root


def survivors(home):
    return sorted(str(p.relative_to(home)) for p in home.rglob("*") if p.is_file())


def test_it_takes_the_live_sessions_jails_and_directories_out(home):
    assert run(home).returncode == 0
    left = survivors(home)
    assert not [p for p in left if "live-" in p], left
    assert (home / "cwd" / "live-switch").exists() is False


def test_it_leaves_everything_else_exactly_where_it_was(home):
    run(home)
    assert "sessions/keepme.jsonl" in survivors(home)
    assert (home / "jails" / "keepme").is_dir()


def test_it_drops_the_pinned_test_dial_but_keeps_real_ones(home):
    from omni.intelligence import registry

    registry.write_cache(["claude"], {"0": {"provider": "claude", "model": "m", "effort": ""}})
    run(home)
    blob = json.loads((home / "cache" / "intelligence.json").read_text())
    assert provider.NAME not in blob
    assert "claude" in blob, "a user's real dial is not this script's business"


def test_a_dry_run_removes_nothing(home):
    before = survivors(home)
    done = run(home, "--dry-run")
    assert "would remove" in done.stdout
    assert survivors(home) == before


def test_it_refuses_a_prefix_that_would_match_everything(home):
    assert run(home, "--prefix", "").returncode != 0


def test_it_refuses_a_prefix_that_is_really_a_path(home, tmp_path):
    """``sessions/../../x`` globs straight out of the home; a prefix is an id."""
    outside = tmp_path / "precious.txt"
    outside.write_text("do not touch")
    done = run(home, "--prefix", f"../../{outside.name[:8]}")
    assert done.returncode != 0
    assert "not a path" in done.stderr
    assert outside.exists()


@pytest.mark.parametrize("prefix", ["*", "?", "live-*", "[a-z]"])
def test_a_prefix_is_a_name_not_a_pattern(home, prefix):
    """``--prefix '*'`` as a glob means the folder itself, and this script deletes
    what it is given. Matching is a plain name comparison so it cannot."""
    before = survivors(home)
    done = run(home, "--prefix", prefix)
    assert done.returncode == 0, done.stderr
    assert survivors(home) == before, f"{prefix!r} removed something"
