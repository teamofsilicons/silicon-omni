"""silicon omni — one interface for Claude Code, Codex and Antigravity.

    from omni import Inference, Event

    chat = Inference.load_or_create_session("my-session")
    chat.model("code")

    @chat.on_event
    def handle(event):
        print(event.type, event.text)

    chat.start()
    chat.send("hello")

The conversation runs in omni's daemon, which keeps every provider warm and
holds the session whether or not this program is attached. It starts itself the
first time anything needs it, so there is nothing to set up.
"""

from .chat import Chat
from .client import DaemonError
from .events import Event
from .inference import Inference, NoAnswer

__version__ = "0.5.0"


class SessionBusy(RuntimeError):
    """No longer raised.

    Until 0.4 a session id could only be held by one process at a time. The
    daemon owns sessions now, so several programs can hold the same one — each
    hears every event, and any of them can send. Kept importable so code that
    catches it still runs.
    """


__all__ = ["Inference", "Event", "Chat", "DaemonError", "SessionBusy", "NoAnswer"]
