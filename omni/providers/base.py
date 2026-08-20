"""What a provider has to be able to do.

Two objects per provider, deliberately kept apart:

``Account``
    Global, session-free: is the CLI here, are we logged in, how much quota is
    left, how do we log in. This is what ``Inference.claude`` hands you.

``Runner``
    One live CLI process driving one session. It runs turns and reports what
    happened as :class:`~omni.events.Event` objects. It does not own history,
    session identity, or model choice — omni does.

Adapters stay dumb on purpose. Everything portable lives above them.
"""

import os
import shutil
from dataclasses import dataclass, field
from typing import Callable

from ..events import CRASH, Event
from ..shared import clock


@dataclass
class Config:
    """Everything a runner needs to know that is not the conversation itself."""

    model: str = ""
    effort: str = ""
    system_prompt: str = ""
    append_system_prompt: str = ""
    disable_subagents: bool = False
    disable_mcp: bool = False
    cwd: str = field(default_factory=os.getcwd)

    def __post_init__(self):
        # Providers resolve symlinks before they name anything after the working
        # directory — on macOS /var and /tmp are links, and an unresolved path
        # makes Claude's session file land somewhere omni will never look again.
        self.cwd = os.path.realpath(self.cwd)


class Account:
    """Installed-ness, auth and quota for one provider. No session involved."""

    name = ""
    cli = ""
    #: seconds to trust a "yes" for. Every provider answers by running its CLI,
    #: which takes seconds, and callers ask more than once.
    ttl = 60.0
    #: seconds to trust a "no" for. Shorter, because a network blip during a
    #: probe would otherwise quietly drop a provider off the dial.
    doubt = 10.0

    def __init__(self):
        self.checked = 0.0
        self.remembered = ""

    @property
    def installed(self) -> bool:
        return shutil.which(self.cli) is not None

    @property
    def auth_status(self) -> str:
        """``"authenticated"`` or ``"unauthenticated"``, remembered briefly."""
        window = self.ttl if self.remembered == "authenticated" else self.doubt
        if self.remembered and clock.epoch() - self.checked < window:
            return self.remembered
        self.remembered = self.probe()
        self.checked = clock.epoch()
        return self.remembered

    def forget(self) -> None:
        """Ask the CLI again next time — after a login, say."""
        self.remembered = ""

    def probe(self) -> str:
        """Ask the CLI whether it is signed in. Implemented per provider."""
        raise NotImplementedError

    @property
    def available(self) -> bool:
        return self.installed and self.auth_status == "authenticated"

    def start_auth(self) -> str:
        """Begin a login. Returns the URL the human has to open."""
        raise NotImplementedError

    def finish_auth(self, code: str) -> str:
        """Complete a login with the code or redirect URL. Returns auth_status."""
        raise NotImplementedError

    @property
    def limits(self):
        """``{"5h": {"used": 0.24, "reset": ts}, "7d": {...}}`` or ``"unauthenticated"``."""
        raise NotImplementedError

    def __repr__(self) -> str:
        return f"<{self.name} {'installed' if self.installed else 'missing'}>"


class Runner:
    """One provider CLI, driving one session.

    Lifecycle: ``start`` (resuming ``native_id`` if given, replaying ``history``
    if not empty) → any number of ``send`` → ``stop``. Everything the model does
    comes back through ``emit``, ending each turn with an ``END`` event.
    """

    name = ""
    cli = ""

    def __init__(self, session_id: str, config: Config, emit: Callable[[Event], None]):
        self.session_id = session_id
        self.config = config
        self.emit = emit
        self.native_id = ""
        self.stopping = False

    def exited(self, code: int) -> None:
        """The CLI is gone. If omni did not ask for that, it is a crash."""
        if self.stopping:
            return
        self.emit(
            Event(
                type=Event.ERROR,
                provider=self.name,
                kind=CRASH,
                ok=False,
                error=f"{self.cli} exited with {code}",
            )
        )

    def start(self, native_id: str = "", history: list[Event] | None = None) -> None:
        """Bring the CLI up.

        ``native_id`` is this provider's own session to resume, if it has one.
        ``history`` is the part of the omni log this provider has not seen and
        must be seeded with — empty when we are simply continuing.
        """
        raise NotImplementedError

    @property
    def seeded(self) -> bool:
        """Has the history handed to :meth:`start` actually reached the provider?

        ``False`` means omni must not mark this provider as caught up yet — a
        runner replaced before it delivers would otherwise skip that history
        forever.
        """
        return True

    def send(self, text: str) -> None:
        """Hand a user message to the CLI, starting or joining a turn."""
        raise NotImplementedError

    def retune(self, model: str, effort: str) -> bool:
        """Change model or effort in place, between turns.

        Return ``True`` if the running process took it. Returning ``False``
        (the default) makes omni restart the provider instead, which is always
        correct but costs re-reading the conversation.
        """
        return False

    def interrupt(self) -> None:
        """Ask the current turn to stop. Best effort."""

    def stop(self) -> None:
        raise NotImplementedError

    @property
    def alive(self) -> bool:
        raise NotImplementedError
