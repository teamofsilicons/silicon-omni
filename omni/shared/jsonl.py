"""Append-only JSONL, the one storage primitive omni uses.

Reads tolerate a torn final line: a reader must never crash on a file that is
being appended to right now.
"""

import json
import threading
from pathlib import Path
from typing import Any, Iterator

WRITE = threading.Lock()


def dumps(obj: Any) -> str:
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))


def append(path: Path, obj: Any) -> None:
    extend(path, [obj])


def extend(path: Path, objs) -> None:
    """One write, so a reader never sees half a batch."""
    objs = list(objs)
    if not objs:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    with WRITE, open(path, "a", encoding="utf-8") as fh:
        fh.write("".join(dumps(o) + "\n" for o in objs))


def stream(path: Path) -> Iterator[dict]:
    if not Path(path).exists():
        return
    with open(path, "r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                continue
