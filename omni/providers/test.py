"""A provider that is not one.

Every other adapter needs a CLI, a login and quota. This one needs nothing,
answers the same way every time, and can be made to fail on cue — so omni's own
behaviour, and yours on top of it, can be driven without spending anything or
waiting on a model::

    from omni import Inference
    from omni.providers import test

    test.install()
    chat = Inference.load_or_create_session("demo", ["test"])
    chat.start()
    chat.send("hello")            # -> TEXT  'echo: hello'

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

Testing the unhappy paths
-------------------------

:func:`running` hands you the live runner, which records what it was given and
what it was sent, and can be driven by hand::

    test.install("alpha", "beta")           # two of them, to switch between
    test.running("alpha").autoreply = False # hold the turn open
    chat.send("hello")
    test.running("alpha").fail("auth")      # now lose the login

Each knob mimics something a real CLI does: ``defer`` is agy, which only sees
history when the next message goes out; ``tunable = False`` is agy again, which
cannot change model without a restart; a native id starting with ``gone-`` is
any provider that has forgotten a session omni thinks it still has.
"""

import re
from typing import Sequence

from ..events import CRASH, Event
from ..intelligence import registry
from ..translate import transcript
from . import base, register

NAME = "test"
TOOL = re.compile(r"\[tool:([^\]\s]+)\]")
RECALL = "[recall]"
LEVELS = 11
PINNED = 10**9  # the dial is generated, not fetched, so it never goes stale

#: The provider that has forgotten the session omni is asking it to resume.
FORGET = "gone-"

#: name -> the runner currently up for it. See :func:`running`.
LIVE: dict[str, "Runner"] = {}


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
        """No quota to report, and ``None`` is how omni says so."""
        return {"5h": {"used": None, "reset": None}, "7d": {"used": None, "reset": None}}


class Runner(base.Runner):
    """One deterministic conversation. In-process: no subprocess, no threads."""

    name = NAME
    cli = "python3"

    #: answer a message as soon as it arrives. Off, and the turn stays open
    #: until you call :meth:`reply` or :meth:`fail` yourself.
    autoreply = True
    #: can change model and effort in place. agy cannot, and says so by not.
    tunable = True
    #: like agy: seeded history only reaches the model with the next message.
    defer = False

    up = False  # so a send before a start says so, rather than raising

    def start(self, native_id: str = "", history=None) -> None:
        if native_id.startswith(FORGET):
            native_id = ""  # this provider no longer knows that session
        self.resumed = bool(native_id)
        self.native_id = native_id or f"{self.name}-{self.session_id}"
        self.given: list[Event] = list(history or [])
        self.sent: list[str] = []
        self.retuned = 0
        self.up = True
        LIVE[self.name] = self

    @property
    def seeded(self) -> bool:
        return not (self.defer and not self.sent)

    def send(self, text: str) -> None:
        if not self.up:
            raise RuntimeError(f"{self.name} is not running; start it first")
        answer = self.answer(text)
        self.sent.append(text)
        if not self.autoreply:
            return
        self.say(Event(type=Event.THINKING))
        for tool in TOOL.findall(text):
            self.ran(tool)
        self.say(Event(type=Event.TEXT, text=answer))
        self.say(Event(type=Event.END, extra={"stop": "complete"}))

    # ------------------------------------------------------------ by hand

    def reply(self, text: str) -> None:
        """Answer and close the turn. For when ``autoreply`` is off."""
        self.say(Event(type=Event.TEXT, text=text))
        self.say(Event(type=Event.END, extra={"stop": "complete"}))

    def fail(self, kind: str = CRASH, error: str = "", ends: bool = False) -> None:
        """Break the way a real CLI breaks.

        ``ends`` adds the ``END`` that all three shipped adapters put out in the
        same breath as the error — the pair, not just the half of it.
        """
        self.say(Event(type=Event.ERROR, kind=kind, ok=False, error=error or f"{kind} from {self.name}"))
        if ends:
            self.say(Event(type=Event.END))

    # ---------------------------------------------------------- internals

    def heard(self) -> list[str]:
        """Everything told to this conversation, seeded history included."""
        return [turn["text"] for turn in transcript(self.given)] + self.sent

    def answer(self, text: str) -> str:
        if RECALL in text:
            return " | ".join(self.heard()) or "nothing yet"
        return f"echo: {text}"

    def ran(self, tool: str) -> None:
        call = f"{self.native_id}-{len(self.sent)}-{tool}"
        self.say(Event(type=Event.TOOL.CALL, tool=tool, id=call, args={"input": tool}))
        self.say(Event(type=Event.TOOL.RESULT, tool=tool, id=call, ok=True, result=f"ran {tool}"))

    def say(self, event: Event) -> None:
        event.provider, event.model = self.name, self.config.model
        self.emit(event)

    def retune(self, model: str, effort: str) -> bool:
        if not self.tunable:
            return False
        self.config.model, self.config.effort = model, effort
        self.retuned += 1
        return self.up

    def stop(self) -> None:
        self.stopping = True
        self.up = False

    @property
    def alive(self) -> bool:
        return self.up


# ------------------------------------------------------------------ setting up

def make(name: str) -> tuple[type, type]:
    """A fresh account/runner pair under a different provider name.

    Two of these is how you test a switch without two vendors.
    """
    return (
        type(f"{name}Account", (Account,), {"name": name}),
        type(f"{name}Runner", (Runner,), {"name": name}),
    )


def rung(provider: str, model: str, effort: str = "") -> dict:
    return {"provider": provider, "model": model, "effort": effort}


def dial(*rungs: dict) -> dict:
    """Spread rungs, best first, over levels 0-10 — the shape the registry serves."""
    steps = len(rungs) - 1
    return {str(level): rungs[round((10 - level) * steps / 10)] for level in range(11)}


def spread(name: str) -> dict:
    """A whole dial on one provider, a different model at every level."""
    return {str(level): rung(name, f"{name}-model-{level}") for level in range(LEVELS)}


def pin(names: Sequence[str], levels: dict) -> None:
    registry.write_cache(list(names), levels, ttl=PINNED)


def install(*names: str, rungs: Sequence[dict] = ()) -> list[str]:
    """Register test providers and pin dials for them, reaching no network.

    With no arguments you get one provider called ``test``, running a different
    model name at every level. Name several and the dial is spread over them,
    strongest first — pass ``rungs`` to say exactly which sits where.
    """
    picked = list(names) or [NAME]
    for name in picked:
        register(name, *make(name))
    if rungs:
        pin(picked, dial(*rungs))
    elif len(picked) == 1:
        pin(picked, spread(picked[0]))
    else:
        pin(picked, dial(*(rung(name, f"{name}-model") for name in picked)))
    # One dial per set of providers, the way a real registry serves them — so a
    # chat that loses one still resolves. Without these, testing a failover
    # dead-ends at NoDial the moment the first provider is dropped.
    for name in picked:
        mine = [r for r in rungs if r["provider"] == name]
        pin([name], dial(*mine) if mine else spread(name))
    return picked


def running(name: str = NAME) -> Runner:
    """The runner currently up for a provider, to inspect or drive by hand."""
    return LIVE[name]
