"""OpenAI, through ``codex app-server``.

Not ``codex exec``: the app server is a long-lived JSON-RPC peer that can start
threads, stream a turn, and — crucially — accept history it never lived through
via ``thread/inject_items``. That is what makes Codex a first-class destination
when a conversation moves.

Codex reads every setting from one folder, so omni gives it an almost empty one
(see :mod:`.jail`) and it has nothing to auto-load.
"""

from .account import Account
from .runner import Runner

__all__ = ["Account", "Runner"]
