"""silicon omni — one interface for Claude Code, Codex and Antigravity.

    from omni import Inference, Event

    chat = Inference.load_or_create_session("my-session")
    chat.intelligence(7)

    @chat.on_event
    def handle(event):
        print(event.type, event.text)

    chat.start()
    chat.send("hello")
"""

from .events import Event
from .inference import Inference
from .intelligence import NoDial
from .session import SessionBusy

__version__ = "0.3.0"
__all__ = ["Inference", "Event", "SessionBusy", "NoDial"]
