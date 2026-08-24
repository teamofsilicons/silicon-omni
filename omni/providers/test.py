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

The double lives in the daemon along with every other provider, so what it
exercises is the real engine over the real socket — not a second one written for
tests. Nothing is registered until :func:`install` is called, so it can never
turn up in ``get_available_providers()`` by accident.

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

import time
from typing import Sequence

from ..client import call

NAME = "test"
RECALL = "[recall]"

#: The provider that has forgotten the session omni is asking it to resume.
FORGET = "gone-"

#: What a knob is called, and what it mimics. See the module docstring.
KNOBS = ("autoreply", "tunable", "defer")


class Live:
    """The runner currently up for one test provider.

    Reads ask the daemon what it has actually seen; writes tell it how to
    behave next. Nothing is cached, so what you read is what happened.
    """

    def __init__(self, name: str):
        # Bypass __setattr__: `name` is ours, not a knob to send anywhere.
        object.__setattr__(self, "name", name)

    # ------------------------------------------------------------ the knobs

    def __getattr__(self, item):
        if item in KNOBS:
            return self.knobs()[item]
        raise AttributeError(item)

    def __setattr__(self, item, value):
        if item not in KNOBS:
            raise AttributeError(f"a test provider has no {item!r}; try {KNOBS}")
        call("test", what="knobs", provider=self.name, value={item: bool(value)})

    def knobs(self) -> dict:
        return call("test", what="knobs", provider=self.name, value={})

    # ---------------------------------------------------------- driving it

    def reply(self, text: str = "") -> None:
        """Answer and close the turn. For when ``autoreply`` is off."""
        call("test", what="reply", provider=self.name, text=text)

    def fail(self, kind: str = "crash", error: str = "", ends: bool = False) -> None:
        """Break the way a real CLI breaks.

        ``ends`` adds the ``END`` that all three shipped adapters put out in the
        same breath as the error — the pair, not just the half of it.
        """
        call(
            "test",
            what="fail",
            provider=self.name,
            value={"kind": kind, "error": error, "ends": ends},
        )

    # --------------------------------------------------------- what it saw

    def state(self) -> dict:
        return call("test", what="state", provider=self.name)

    @property
    def sent(self) -> list[str]:
        """Every message handed to this provider, in order."""
        return self.state()["sent"]

    @property
    def heard(self) -> list[str]:
        """Everything told to this conversation, seeded history included."""
        return self.state()["heard"]

    @property
    def resumed(self) -> bool:
        """Was it given a native session to pick back up?"""
        return self.state()["resumed"]

    @property
    def retuned(self) -> int:
        """How many times its model was changed without a restart."""
        return self.state()["retuned"]

    @property
    def up(self) -> bool:
        return self.state()["up"]

    def __repr__(self) -> str:
        return f"<test provider {self.name!r}>"


def rung(provider: str, model: str, effort: str = "") -> dict:
    return {"provider": provider, "model": model, "effort": effort}


def install(*names: str, rungs: Sequence[dict] = ()) -> list[str]:
    """Register test providers and pin dials for them, reaching no network.

    With no arguments you get one provider called ``test``, running a different
    model name at every level. Name several and the dial is spread over them,
    strongest first — pass ``rungs`` to say exactly which sits where.
    """
    return call(
        "test",
        what="install",
        providers=list(names),
        value={"rungs": [dict(item) for item in rungs]},
    )


def running(name: str = NAME, timeout: float = 10.0) -> Live:
    """The runner currently up for a provider, to inspect or drive by hand.

    A provider comes up on the daemon's own thread, a moment after the session
    that wants it opens. So this waits for it rather than racing it — which is
    what ``test.running("alpha").autoreply = False`` on the line after
    ``chat.start()`` needs to mean.
    """
    from ..client import DaemonError

    deadline = time.time() + timeout
    while True:
        try:
            call("test", what="state", provider=name)
            return Live(name)
        except DaemonError:
            if time.time() >= deadline:
                raise
            time.sleep(0.02)


def installed() -> list[str]:
    """Which test providers are registered right now."""
    return call("test", what="installed")


def forget_all() -> None:
    """Unregister every test provider. Between test runs."""
    call("test", what="forget_all")
