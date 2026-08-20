"""One 0-10 dial that spans every provider you are logged into.

A level maps to ``{"provider": ..., "model": ..., "effort": ...}``, and omni
hands those two strings to the CLI verbatim. It does not interpret them, does
not rank anything, and does not know the name of a single model. Working out
which models belong on the dial happens at the registry, over a list kept in a
public repo, so a model released tomorrow needs no release of this package.

The answer is kept under ``~/.omni/cache`` for an hour. Nothing is shipped in
the wheel as a fallback: a model list baked into a release is a model list that
goes quietly stale, and a wrong recommendation is worse than an honest refusal.
A dial that was fetched once is reused even after it expires, so a machine that
has run before keeps working offline.
"""

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from ..shared import clock, paths

#: Where the dial comes from unless you say otherwise.
REGISTRY = "https://omni.teamofsilicons.com/intelligence.json"
CACHE_TTL = 60 * 60  # burst the cache after an hour
QUIET_TTL = 5 * 60  # after a failed fetch, sit still rather than retry every call
VERSION = 2  # bump when a rung's shape changes; older caches are then ignored
LEVELS = 11  # 0..10


class NoDial(LookupError):
    """The registry has never been reached, so there is nothing to route to."""


def remote() -> str:
    """The registry to ask. Read per call, so ``OMNI_REGISTRY`` can be set late."""
    return os.environ.get("OMNI_REGISTRY") or REGISTRY


def key(providers) -> str:
    """One dial per set of providers, named the same way on both sides."""
    return "+".join(sorted(providers))


def is_dial(value) -> bool:
    """A dial is levels to rungs. Anything else is an envelope we are inside."""
    return (
        isinstance(value, dict)
        and bool(value)
        and all(str(level).isdigit() for level in value)
        and all(isinstance(rung, dict) and "model" in rung for rung in value.values())
    )


def unwrap(payload, name: str):
    """Take the dial out of whatever the registry wrapped it in.

    Checked rather than assumed: an envelope that happens not to contain our
    key must not be mistaken for a dial and cached as one.
    """
    if not isinstance(payload, dict):
        return None
    wrapped = payload.get("ladders")
    for candidate in (
        wrapped.get(name) if isinstance(wrapped, dict) else None,
        payload.get("ladder"),
        payload,
    ):
        if is_dial(candidate):
            return candidate
    return None


def fetch(name: str, timeout: float = 5.0):
    """Ask the registry for the dial for exactly these providers."""
    url = f"{remote()}?{urllib.parse.urlencode({'providers': name})}"
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return unwrap(json.loads(response.read().decode("utf-8")), name)
    except (urllib.error.URLError, TimeoutError, ValueError, OSError):
        return None


def cache_file() -> Path:
    return paths.cache() / "intelligence.json"


def cached() -> dict:
    try:
        blob = json.loads(cache_file().read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return blob if blob.get("version") == VERSION else {}


def read_cache(name: str, fresh_only: bool = True):
    entry = cached().get(name)
    if not entry:
        return None
    if fresh_only and clock.epoch() - entry.get("at", 0) > entry.get("ttl", CACHE_TTL):
        return None
    return entry.get("levels")


def write_cache(providers, levels: dict, ttl: float = CACHE_TTL) -> None:
    paths.ensure(paths.cache())
    blob = cached() or {"version": VERSION}
    name = providers if isinstance(providers, str) else key(providers)
    blob[name] = {"at": clock.epoch(), "ttl": ttl, "levels": levels}
    cache_file().write_text(json.dumps(blob))


def levels(providers) -> dict:
    """The dial for these providers, from the registry or from what it said last."""
    name = key(providers)
    fresh = read_cache(name)
    if fresh is not None:
        return fresh
    found = fetch(name)
    if found is not None:
        write_cache(name, found)
        return found
    # Unreachable. An old answer beats no answer, but stop asking for a while
    # rather than stalling on every call.
    stale = read_cache(name, fresh_only=False)
    if stale is None:
        return {}
    write_cache(name, stale, QUIET_TTL)
    return stale


def table(providers) -> dict[int, dict]:
    """Levels 0-10 for the providers you have, 10 being the best you can reach."""
    return {int(level): dict(rung, level=int(level)) for level, rung in levels(providers).items()}


def resolve(level: int, providers) -> dict:
    """The single rung for ``level``. Out-of-range levels clamp rather than raise."""
    rungs = table(providers)
    if not rungs:
        raise NoDial(
            f"no dial for {sorted(providers)}: could not reach {remote()} and nothing is cached"
        )
    return rungs[min(max(int(level), 0), LEVELS - 1)]
