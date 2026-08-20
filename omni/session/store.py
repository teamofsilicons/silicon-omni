"""The session file: an append-only log of :class:`Event` objects.

The event stream and the conversation history are the same thing. Nothing is
summarised on the way in and nothing is dropped, so a session can always be
replayed into a provider that was not there when it happened.
"""

from ..events import HISTORY_TYPES, Event
from ..shared import jsonl, paths


class Store:
    def __init__(self, session_id: str):
        self.session_id = session_id
        self.path = paths.session_file(session_id)
        self.seq = max((e.get("seq", -1) for e in jsonl.stream(self.path)), default=-1)

    def __len__(self) -> int:
        return self.seq + 1

    @property
    def exists(self) -> bool:
        return self.path.exists()

    def append(self, event: Event) -> Event:
        """Stamp the event with its position in the session and persist it."""
        self.seq += 1
        event.seq = self.seq
        event.session = event.session or self.session_id
        jsonl.append(self.path, event.to_dict())
        return event

    def events(self, since: int = 0) -> list[Event]:
        return [Event.from_dict(d) for d in jsonl.stream(self.path) if d.get("seq", 0) >= since]

    def history(self, since: int = 0) -> list[Event]:
        """Just the conversation: what a provider needs to be brought up to date."""
        return [e for e in self.events(since) if e.type in HISTORY_TYPES]
