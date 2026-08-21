"""A provider that is not one.

Every other adapter needs a CLI, a login and quota. This one needs nothing and
answers the same way every time, so omni's own behaviour — seeding, switching,
injection, sync marks — can be driven without spending anything or waiting on a
model::

    from omni import Inference
    from omni.providers import test

    test.install()
    chat = Inference.load_or_create_session("demo", ["test"])
    chat.start()
    chat.send("hello")            # -> TEXT "echo: hello"

Nothing is registered until :func:`install` is called, so it can never turn up
in ``get_available_providers()`` by accident.

What it does with what you send:

======================  ======================================================
``[tool:NAME]``         runs ``NAME``: a ``TOOL.CALL`` and a matching result
``[recall]``            replies with everything it was told before this message
anything else           replies ``echo: <what you sent>``
======================  ======================================================

``[recall]`` is the interesting one: it answers out of history it was *seeded*
with as readily as history it lived through, which is exactly what a provider
switch has to preserve.
"""

import re

from ..events import Event
from ..intelligence import registry
from ..translate import transcript
from . import base, register

NAME = "test"
TOOL = re.compile(r"\[tool:([^\]\s]+)\]")
RECALL = "[recall]"
LEVELS = 11
PINNED = 10**9  # the dial is generated, not fetched, so it never goes stale


class Account(base.Account):
    """Always here, always signed in, never rate limited."""

    name = NAME
    cli = "python3"

    @property
    def installed(self) -> bool:
        return True

    def probe(self) -> str:
        return "authenticated"

    def start_auth(self) -> str:
        return "the test provider needs no login"

    def finish_auth(self, code: str = "") -> str:
        return self.auth_status

    @property
    def limits(self):
        """No quota to report. ``None`` is unknown, and unknown is the truth here."""
        return {"5h": {"used": 0.0, "reset": None}, "7d": {"used": 0.0, "reset": None}}


class Runner(base.Runner):
    """One deterministic conversation. In-process: no subprocess, no threads."""

    name = NAME
    cli = "python3"
    up = False  # so a send before a start says so, rather than raising an attribute error

    def start(self, native_id: str = "", history=None) -> None:
        self.native_id = native_id or f"{NAME}-{self.session_id}"
        self.heard = [turn["text"] for turn in transcript(history or [])]
        self.up = True

    def send(self, text: str) -> None:
        if not self.up:
            raise RuntimeError("the test provider is not running; start it first")
        self.say(Event(type=Event.THINKING))
        for tool in TOOL.findall(text):
            self.ran(tool)
        answer = self.answer(text)
        self.heard.append(text)
        self.say(Event(type=Event.TEXT, text=answer))
        self.say(Event(type=Event.END, extra={"stop": "complete"}))

    def answer(self, text: str) -> str:
        if RECALL in text:
            return " | ".join(self.heard) or "nothing yet"
        return f"echo: {text}"

    def ran(self, tool: str) -> None:
        call = f"{self.native_id}-{len(self.heard)}-{tool}"
        self.say(Event(type=Event.TOOL.CALL, tool=tool, id=call, args={"input": tool}))
        self.say(Event(type=Event.TOOL.RESULT, tool=tool, id=call, ok=True, result=f"ran {tool}"))

    def say(self, event: Event) -> None:
        event.provider, event.model = self.name, self.config.model
        self.emit(event)

    def retune(self, model: str, effort: str) -> bool:
        self.config.model, self.config.effort = model, effort
        return self.up

    def stop(self) -> None:
        self.stopping = True
        self.up = False

    @property
    def alive(self) -> bool:
        return self.up


def dial(levels: int = LEVELS) -> dict:
    """A whole 0-10 dial on this provider alone, so routing runs unchanged."""
    return {
        str(level): {"provider": NAME, "model": f"{NAME}-model-{level}", "effort": ""}
        for level in range(levels)
    }


def install() -> str:
    """Make the provider usable, and pin its dial so nothing reaches the network."""
    register(NAME, Account, Runner)
    registry.write_cache([NAME], dial(), ttl=PINNED)
    return NAME
