"""Claude Code's own session files, which is how omni seeds it.

Claude keeps one JSONL per session at ``~/.claude/projects/<slug>/<uuid>.jsonl``
where the slug is the working directory with every non-alphanumeric character
turned into a dash. Records form a linked list through ``parentUuid``.

Writing that file before ``--resume`` is how a conversation that happened
somewhere else becomes one Claude remembers. Appending to a file Claude already
owns works just as well, so coming back only ever costs the part it missed.
"""

import json
import re
import uuid
from pathlib import Path

from ...shared import clock, jsonl

VERSION = "2.1.237"


def home() -> Path:
    return Path.home() / ".claude" / "projects"


def slug(cwd: str) -> str:
    return re.sub(r"[^a-zA-Z0-9]", "-", str(cwd))


def session_path(cwd: str, session_id: str) -> Path:
    return home() / slug(cwd) / f"{session_id}.jsonl"


def tail_uuid(path: Path) -> str | None:
    """The last record's uuid — what the next record has to hang off."""
    last = None
    for record in jsonl.stream(path):
        if record.get("uuid"):
            last = record["uuid"]
    return last


def record(kind: str, cwd: str, session_id: str, parent: str | None, message: dict) -> dict:
    return {
        "parentUuid": parent,
        "isSidechain": False,
        "userType": "external",
        "cwd": str(cwd),
        "sessionId": session_id,
        "version": VERSION,
        "gitBranch": "",
        "type": kind,
        "uuid": str(uuid.uuid4()),
        "timestamp": clock.now(),
        "message": message,
    }


def seed(cwd: str, session_id: str, turns: list[dict], model: str) -> Path:
    """Append a transcript to Claude's session file, creating it if needed.

    ``turns`` is what :func:`omni.translate.transcript` produced: a list of
    ``{"role", "text"}``.
    """
    path = session_path(cwd, session_id)
    path.parent.mkdir(parents=True, exist_ok=True)
    parent = tail_uuid(path)
    written = []
    for turn in turns:
        if turn["role"] == "user":
            item = record("user", cwd, session_id, parent, {"role": "user", "content": turn["text"]})
        else:
            item = record(
                "assistant",
                cwd,
                session_id,
                parent,
                {
                    "id": f"msg_{uuid.uuid4().hex[:16]}",
                    "type": "message",
                    "role": "assistant",
                    "model": model or "claude-sonnet-5",
                    "content": [{"type": "text", "text": turn["text"]}],
                    "stop_reason": None,
                    "stop_sequence": None,
                    "usage": {"input_tokens": 1, "output_tokens": 1},
                },
            )
            item["requestId"] = "req_omni_seed"
        parent = item["uuid"]
        written.append(item)
    jsonl.extend(path, written)
    return path


def user_line(text: str) -> str:
    """One stdin line for ``--input-format stream-json``."""
    return json.dumps(
        {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}
    )
