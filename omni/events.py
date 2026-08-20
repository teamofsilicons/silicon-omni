"""The event vocabulary.

Everything omni has to say arrives as an :class:`Event`. The same objects are
handed to ``@chat.on_event`` handlers, to ``@chat.logs`` handlers, and appended
to the session file — so the session file *is* the event log, and there is only
ever one schema to learn.

Reasoning is deliberately contentless: a ``THINKING`` event says the model is
thinking, never what it thought. Provider reasoning is encrypted or signed and
cannot be replayed into another provider, so omni does not carry it around.
"""

from dataclasses import dataclass, field
from typing import Any

from .shared import clock


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
    ``NEW_SESSION``     ``session``, ``extra['native']``
    ``CONFIG``          ``text`` — what changed, ``extra`` — the new value
    ==================  ====================================================

    Three fields are on every event whatever its type. ``session`` is the omni
    session it belongs to, ``at`` is when it happened, and ``seq`` is its
    position in that session's log — a number that only goes up, and never
    repeats, across every provider the conversation has passed through.

    ``seq`` is how omni knows what a provider still has to be told: the meta
    file records the last one each provider saw, so coming back to one replays
    exactly the events recorded since, and nothing twice.

    ``CONFIG`` is the catch-all for settings and bookkeeping. Its ``text`` says
    which: ``launch``, ``retune``, ``reseed``, ``stop``, ``provider_removed``,
    ``unsupported``, ``approximated``, or the name of whatever call you made.
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
    at: str = field(default_factory=clock.now)
    seq: int = -1
    extra: dict = field(default_factory=dict)

    def to_dict(self) -> dict:
        """Compact dict for the session file: fields still at their default are dropped."""
        return {
            name: value
            for name, value in vars(self).items()
            if name in ALWAYS or value != DEFAULTS[name]
        }

    @classmethod
    def from_dict(cls, data: dict) -> "Event":
        known = {f for f in cls.__dataclass_fields__}
        return cls(**{k: v for k, v in data.items() if k in known})


#: Written even when unset: ``type`` identifies the record, ``at`` orders it.
ALWAYS = ("type", "at")

#: Every field's default, materialised once so ``to_dict`` stays cheap.
DEFAULTS = vars(Event(type=""))

#: Event types that carry conversation content, i.e. the ones replayed into a
#: provider when a session is seeded. Everything else is bookkeeping.
HISTORY_TYPES = (
    Event.START,
    Event.INJECTED,
    Event.TEXT,
    Event.THINKING,
    Tool.CALL,
    Tool.RESULT,
)

#: Error classifications used by ``Event.kind``.
AUTH = "auth"
LIMIT = "limit"
UNAVAILABLE = "unavailable"
CRASH = "crash"

FAULTS = (
    (AUTH, ("auth", "unauthorized", "401", "login", "sign in")),
    (LIMIT, ("rate", "limit", "quota", "429")),
    (UNAVAILABLE, ("overload", "unavailable", "disconnect", "timeout", "503", "502")),
)


def classify(text) -> str:
    """Which kind of failure this is, in omni's vocabulary.

    Every provider words its failures differently; omni only cares whether you
    need to log in, wait, retry, or look at a stack trace.
    """
    lowered = str(text).lower()
    for kind, words in FAULTS:
        if any(word in lowered for word in words):
            return kind
    return CRASH
