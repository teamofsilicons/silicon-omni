"""The front door.

``Inference`` is the only object you need to import to use omni: it tells you
which providers are usable, hands you sessions, and exposes each provider's
account for auth and quota.

Every one of those answers comes from the daemon, which already has the CLIs
warm — so asking is a socket round trip rather than a process launch.
"""

from typing import Sequence

from .chat import Chat
from .client import DaemonError, call


class NoDial(LookupError):
    """The registry has never been reached for the requested providers."""


class ProviderHandle:
    """One provider's account: is it here, are we logged in, what is left.

    Nothing is fetched until you ask for it, so ``Inference.claude`` costs
    nothing to mention.
    """

    def __init__(self, name: str):
        self.name = name

    @property
    def installed(self) -> bool:
        return bool(call("account", provider=self.name, what="installed"))

    @property
    def auth_status(self) -> str:
        """``"authenticated"`` or ``"unauthenticated"``."""
        return call("account", provider=self.name, what="auth_status")

    @property
    def available(self) -> bool:
        return self.installed and self.auth_status == "authenticated"

    @property
    def limits(self):
        """``{"5h": {"used": 0.24, "reset": iso}, "7d": {...}}`` or ``"unauthenticated"``.

        ``used`` is a fraction (``0.24`` is 24%) and ``reset`` is an RFC3339 UTC
        string whatever the provider natively answers in. Either may be ``None``:
        some plans report no windows at all, and "nobody said" is a different
        thing from "you have spent nothing".
        """
        return call("account", provider=self.name, what="limits", timeout=180.0)

    def start_auth(self) -> str:
        """Begin a login. Returns the URL to open in a browser."""
        return call("account", provider=self.name, what="start_auth", timeout=180.0)

    def finish_auth(self, code: str = "") -> str:
        """Complete a login with the code or redirect URL. Returns the auth status."""
        return call(
            "account", provider=self.name, what="finish_auth", text=code, timeout=600.0
        )

    def forget(self) -> None:
        """Ask the CLI again next time, rather than trusting what it last said."""
        call("account", provider=self.name, what="forget")

    def __repr__(self) -> str:
        return f"<{self.name}>"


class Inference:
    #: Anthropic, through the ``claude`` CLI.
    claude = ProviderHandle("claude")
    #: OpenAI, through ``codex app-server``.
    openai = ProviderHandle("openai")
    #: Google, through Antigravity's ``agy``.
    google = ProviderHandle("google")

    @staticmethod
    def get_available_providers(limit_to: Sequence[str] | None = None) -> list[str]:
        """Providers whose CLI is installed *and* logged in.

        Pass ``limit_to`` to narrow the set omni is allowed to use.
        """
        return call("providers", providers=list(limit_to) if limit_to else None, timeout=180.0)

    @staticmethod
    def dial(providers: Sequence[str] | None = None) -> dict:
        """What each intelligence level 0-10 means for a set of providers.

        ``{"0": {"provider": ..., "model": ..., "effort": ...}, ...}``, straight
        from the registry omni routes by. Handy for showing a user what they are
        about to spend, and for finding the level that lands on a given
        provider.
        """
        try:
            return call(
                "dial", providers=list(providers) if providers else None, timeout=60.0
            )
        except DaemonError as exc:
            # ``NoDial`` was public before the daemon split. The wire protocol
            # only carries an error sentence today, so translate precisely the
            # refusal the dial endpoint owns and leave every other daemon
            # failure as ``DaemonError``.
            if str(exc).startswith("no dial for "):
                raise NoDial(str(exc)) from exc
            raise

    @staticmethod
    def load_or_create_session(session_id: str, providers: Sequence[str] | None = None) -> Chat:
        """Open a session by id, creating it if this is the first time.

        Several programs may hold the same session at once: each hears every
        event, and any of them can send. The conversation belongs to the daemon,
        not to whoever happens to be attached.
        """
        return Chat(session_id, list(providers) if providers else [])

    @staticmethod
    def sessions() -> list[dict]:
        """Every session the daemon is currently holding open."""
        return call("sessions").get("sessions", [])

    @staticmethod
    def daemon() -> dict:
        """What the daemon says about itself: version, pid, home."""
        return call("ping")


__all__ = ["Inference", "ProviderHandle", "DaemonError", "NoDial"]
