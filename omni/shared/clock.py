"""Timestamps. One format everywhere: RFC3339 UTC with a trailing Z."""

from datetime import datetime, timezone


def now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def epoch() -> float:
    return datetime.now(timezone.utc).timestamp()
