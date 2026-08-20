"""One 0-10 dial that spans every provider you are logged into.

A level maps to ``{"provider": ..., "model": ..., "effort": ...}``. omni hands
those two strings to the CLI verbatim and never interprets them, so a new model
is new data and no new code.

The dial arrives finished. It is already reduced to the leftmost models on
GDPval's score-versus-price graph — level 10 at the top, walking down and to the
left, so a step down the dial is always cheaper and never a sideways move. omni
asks ``omni.teamofsilicons.com`` for the dial matching the providers it has,
keeps the answer under ``~/.omni/cache`` for an hour, and falls back to the
packaged :file:`ladder.json`. ``OMNI_REGISTRY`` points it somewhere else.

Working out which models belong on the dial is not omni's job — see
``tools/build_ladder.py``, which is what produces that file.
"""

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from ..shared import clock, paths

#: Where the dial comes from unless you say otherwise.
REMOTE = "https://omni.teamofsilicons.com/api/intelligence"
CACHE_TTL = 60 * 60  # burst the cache after an hour
QUIET_TTL = 5 * 60  # after a failed fetch, sit still rather than retry every call
VERSION = 1  # bump when a rung's shape changes; older caches are then ignored
LEVELS = 11  # 0..10
LADDER_FILE = Path(__file__).with_name("ladder.json")


def remote() -> str:
    """The registry to ask. Read per call, so ``OMNI_REGISTRY`` can be set late."""
    return os.environ.get("OMNI_REGISTRY") or REMOTE


def key(providers) -> str:
    """One dial per set of providers, named the same way on both sides."""
    return "+".join(sorted(providers))


def packaged(name: str) -> dict:
    doc = json.loads(LADDER_FILE.read_text(encoding="utf-8"))
    return doc.get("ladders", {}).get(name) or {}


def unwrap(payload, name: str):
    """Take the dial out of whatever upstream wrapped it in."""
    if not isinstance(payload, dict):
        return None
    inside = payload.get("ladders", {}).get(name) or payload.get("ladder") or payload
    return inside if isinstance(inside, dict) and inside else None


def fetch(name: str, timeout: float = 5.0):
    """Ask upstream for the dial for exactly these providers."""
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


def read_cache(name: str, fresh_only: bool = True, source: str = ""):
    entry = cached().get(name)
    if not entry:
        return None
    if source and entry.get("source") != source:
        return None
    if fresh_only and clock.epoch() - entry.get("at", 0) > entry.get("ttl", CACHE_TTL):
        return None
    return entry.get("levels")


def write_cache(providers, levels: dict, ttl: float = CACHE_TTL, source: str = "remote") -> None:
    paths.ensure(paths.cache())
    blob = cached() or {"version": VERSION}
    blob[key(providers) if not isinstance(providers, str) else providers] = {
        "at": clock.epoch(),
        "ttl": ttl,
        "source": source,
        "levels": levels,
    }
    cache_file().write_text(json.dumps(blob))


def levels(providers) -> dict:
    """The dial for these providers: remote if reachable, packaged otherwise."""
    name = key(providers)
    fresh = read_cache(name)
    if fresh is not None:
        return fresh
    found = fetch(name)
    if found is None:
        # A dial we once fetched beats the one we shipped. Either way, stop
        # asking for a few minutes rather than stalling on every call.
        found = read_cache(name, fresh_only=False, source="remote") or packaged(name)
        write_cache(name, found, QUIET_TTL, "packaged")
    else:
        write_cache(name, found, CACHE_TTL, "remote")
    return found


def table(providers) -> dict[int, dict]:
    """Levels 0-10 for the providers you have, 10 being the best you can reach."""
    return {int(level): dict(rung, level=int(level)) for level, rung in levels(providers).items()}


def resolve(level: int, providers) -> dict:
    """The single rung for ``level``. Out-of-range levels clamp rather than raise."""
    rungs = table(providers)
    if not rungs:
        raise LookupError(f"no dial available for providers {sorted(providers)}")
    return rungs[min(max(int(level), 0), LEVELS - 1)]
