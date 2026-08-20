"""Claude Code.

Flags do all the work here: subagents, MCP, slash commands and memory files can
all be switched off from the command line, and ``--session-id`` lets omni pick
the session uuid up front. Seeding is a file write — see :mod:`.session`.
"""

from .account import Account
from .runner import Runner

__all__ = ["Account", "Runner"]
