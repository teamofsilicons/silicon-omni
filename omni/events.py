"""The event vocabulary.

Everything omni has to say arrives as an :class:`Event`. Daemon events are handed
to ``@chat.on_event`` and ``@chat.logs`` handlers and appended to the session
file — so the session file *is* the event log, and there is only one schema to
learn. A local ``handler`` error additionally reaches this Python client's log
handlers; it has no sequence number because it is not a daemon record.

The daemon defines these; this is the Python view of the same thing, so an
event read off a session file and an event handed to a callback are identical.

Reasoning is deliberately contentless: a ``THINKING`` event says the model is
thinking, never what it thought. Provider reasoning is encrypted or signed and
cannot be replayed into another provider, so omni does not carry it around.
"""

from dataclasses import dataclass, field
from typing import Any


class Tool:
    """Namespace so ``Event.TOOL.CALL`` reads the way it should."""

    CALL = "tool.call"
    RESULT = "tool.result"


@dataclass
class Event:
    """One thing that happened.

    Class attributes are the event *types*, instance fields are the payload::

        @chat.on_event
        def handle(event):
            if event.type == Event.TOOL.CALL:
                print(event.tool, event.args)

    Which fields are populated depends on ``type``:

    ==================  ====================================================
    type                carries
    ==================  ====================================================
    ``START``           ``text`` — the user message that opened this turn
    ``TEXT``            ``text`` — one completed assistant message block
    ``THINKING``        nothing; the model is reasoning (never the content)
    ``TOOL.CALL``       ``tool``, ``args``, ``id``
    ``TOOL.RESULT``     ``tool``, ``id``, ``result``, ``ok``
    ``END``             ``extra`` — stop reason, usage, if the provider says
    ``INJECTED``        ``text`` — a message that landed mid-turn
    ``ERROR``           ``error``, ``kind`` — one of ``auth`` / ``limit`` /
                        ``unavailable`` / ``crash`` from the model or its CLI,
                        ``stderr`` for CLI chatter, ``omni`` when the engine
                        itself failed, ``handler`` when your callback raised
    ``SWITCH_PROVIDER`` ``provider`` (the new one), ``extra['from']``
    ``NEW_SESSION``     ``provider``, ``extra['native']``
    ``CONFIG``          ``text`` — what changed, ``extra`` — the new value
    ==================  ====================================================

    Every serialized event carries ``v``, ``type``, and ``at``. ``v`` is the
    on-disk schema version (currently 1); a pre-versioned record defaults to 1
    when read. Events committed by the daemon also carry ``session`` and
    ``seq``. The latter is a position in that session's log that only goes up
    and never repeats across every provider the conversation has visited.

    ``seq`` is how omni knows what a provider still has to be told, and how a
    client that reconnects asks for exactly what it missed. ``turn`` starts at
    zero on the first ``START`` and groups the events belonging to that omni
    turn; bookkeeping before it is omitted. ``native`` is an optional map of
    provider-reported identities — conversation, message, turn, item, tool, or
    step IDs — retained for exact fidelity but not translated as portable
    history.
    """

    # ---- types ----
    START = "start"
    TEXT = "text"
    THINKING = "thinking"
    TOOL = Tool
    END = "end"
    INJECTED = "injected"
    ERROR = "error"
    SWITCH_PROVIDER = "switch_provider"
    NEW_SESSION = "new_session"
    CONFIG = "config"

    # ---- payload ----
    type: str
    v: int = 1
    session: str = ""
    provider: str = ""
    model: str = ""
    text: str = ""
    tool: str = ""
    id: str = ""
    args: dict = field(default_factory=dict)
    result: Any = None
    ok: bool = True
    kind: str = ""
    error: str = ""
    at: str = ""
    seq: int = -1
    turn: int = -1
    native: dict = field(default_factory=dict)
    extra: dict = field(default_factory=dict)

    @classmethod
    def from_dict(cls, data: dict) -> "Event":
        """Build one from what the daemon sent. Unknown fields are ignored."""
        known = set(cls.__dataclass_fields__)
        values = {key: value for key, value in data.items() if key in known}
        return cls(**values)

    def to_dict(self) -> dict:
        """Fields still at their default are dropped, as on the wire."""
        return {
            name: value
            for name, value in vars(self).items()
            if name in ALWAYS or value != DEFAULTS[name]
        }


#: Always written: ``v`` selects the schema, ``type`` identifies, ``at`` orders.
ALWAYS = ("v", "type", "at")

#: Every field's default, materialised once so ``to_dict`` stays cheap.
DEFAULTS = vars(Event(type=""))

#: Error classifications used by ``Event.kind``.
AUTH = "auth"
LIMIT = "limit"
UNAVAILABLE = "unavailable"
CRASH = "crash"
