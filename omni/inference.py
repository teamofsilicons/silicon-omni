"""The front door.

``Inference`` is the only object you need to import to use omni: it tells you
which providers are usable, hands you sessions, and exposes each provider's
account for auth and quota.
"""

from typing import Sequence

from . import providers
from .chat import Chat


class ProviderHandle:
    """Lazy attribute access so ``Inference.claude`` never imports codex."""

    def __init__(self, name: str):
        self.name = name

    def __getattr__(self, item):
        return getattr(providers.account(self.name), item)

    def __repr__(self) -> str:
        return repr(providers.account(self.name))


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
        return providers.available(limit_to)

    @staticmethod
    def load_or_create_session(session_id: str, providers_: Sequence[str] | None = None) -> Chat:
        """Open a session by id, creating it if this is the first time.

        Raises :class:`~omni.session.SessionBusy` if another live process
        already holds that id — one chat per session, always.
        """
        return Chat(session_id, providers_ or providers.available())
