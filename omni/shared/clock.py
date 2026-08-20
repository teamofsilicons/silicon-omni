"""Timestamps. One format everywhere: RFC3339 UTC with a trailing Z.

Providers do not agree on how to say "at". Claude answers in ISO, Codex in ISO
with an offset, agy in seconds since the epoch — and any of them could change
their mind in a release. :func:`iso` is the one funnel everything goes through,
so callers only ever read one shape.
"""

from datetime import datetime, timezone


def stamp(when: datetime) -> str:
    return when.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def now() -> str:
    return stamp(datetime.now(timezone.utc))


def epoch() -> float:
    return datetime.now(timezone.utc).timestamp()


def iso(value):
    """Whatever a provider called a moment, as one RFC3339 UTC string.

    Seconds, milliseconds and ISO strings with any offset all land in the same
    shape. ``None`` stays ``None`` — unknown is not the epoch — and something
    unparseable is handed back untouched rather than turned into a wrong time.
    """
    if value is None or value == "":
        return None
    text = str(value).strip()
    try:
        seconds = float(text)
    except ValueError:
        pass
    else:
        # Milliseconds, if the number is far past any plausible second count.
        try:
            return stamp(datetime.fromtimestamp(seconds / 1000 if seconds > 1e11 else seconds, timezone.utc))
        except (OverflowError, OSError, ValueError):
            return text
    try:
        when = datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        return text
    return stamp(when if when.tzinfo else when.replace(tzinfo=timezone.utc))
