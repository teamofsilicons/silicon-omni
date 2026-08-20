"""Every path omni owns, in one place.

Set ``OMNI_HOME`` to relocate the whole tree (tests do exactly this).
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
    """Which native session each provider holds for this omni session, and how far it is synced."""
    return sessions() / f"{session_id}.meta.json"


def lock_file(session_id: str) -> Path:
    return sessions() / f"{session_id}.lock"


def jail(session_id: str, provider: str) -> Path:
    """A fake provider home, used to strip a CLI of everything it would otherwise auto-load."""
    return home() / "jails" / session_id / provider


def cache() -> Path:
    return home() / "cache"


def ensure(path: Path) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    return path
