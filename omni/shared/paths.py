"""Where omni keeps things.

One tree, shared by the daemon and every client. Set ``OMNI_HOME`` to move it —
the daemon is started with whatever the client had, so a test home and a real
one never meet.
"""

import os
from pathlib import Path


def home() -> Path:
    """Root of omni's state. Read on every call so tests can move it at runtime."""
    return Path(os.environ.get("OMNI_HOME") or Path.home() / ".omni")


def sessions() -> Path:
    return home() / "sessions"


def session_file(session_id: str) -> Path:
    """The source of truth: full cross-provider history for one omni session."""
    return sessions() / f"{session_id}.jsonl"


def meta_file(session_id: str) -> Path:
    """Which native session each provider holds, and how far it is synced."""
    return sessions() / f"{session_id}.meta.json"


def socket() -> Path:
    """Where the daemon listens. One per ``OMNI_HOME``."""
    return home() / "omnid.sock"


def cache() -> Path:
    return home() / "cache"


def ensure(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path
