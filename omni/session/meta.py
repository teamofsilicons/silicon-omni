"""Which native session each provider holds for one omni session.

omni session A may sit on claude session B and codex thread C. ``synced`` is how
far up the omni log that native session has already seen, so coming back to a
provider only replays the part it missed.
"""

import json
import os

from ..shared import clock, paths


class Meta:
    def __init__(self, session_id: str):
        self.session_id = session_id
        self.path = paths.meta_file(session_id)
        self.data = self.load() or {
            "session": session_id,
            "created": clock.now(),
            "providers": {},
        }

    def load(self) -> dict | None:
        """A torn or hand-mangled file starts over rather than bricking the session."""
        try:
            return json.loads(self.path.read_text())
        except (OSError, json.JSONDecodeError):
            return None

    def save(self) -> None:
        """Written whole or not at all: this is rewritten on every turn."""
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.data["updated"] = clock.now()
        staging = self.path.with_suffix(f".tmp.{os.getpid()}")
        staging.write_text(json.dumps(self.data, indent=2))
        os.replace(staging, self.path)

    def native(self, provider: str) -> dict:
        """``{"id": <native session id>, "synced": <omni seq already replayed>}``."""
        return self.data["providers"].setdefault(provider, {"id": "", "synced": -1})

    def bind(self, provider: str, native_id: str) -> None:
        self.native(provider)["id"] = native_id
        self.save()

    def mark_synced(self, provider: str, seq: int) -> None:
        self.native(provider)["synced"] = seq
        self.save()

    def get(self, key: str, default=None):
        return self.data.get(key, default)

    def set(self, key: str, value) -> None:
        self.data[key] = value
        self.save()
