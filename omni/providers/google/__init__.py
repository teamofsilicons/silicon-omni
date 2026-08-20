"""Google, through Antigravity's ``agy``.

The plainest of the three and the most restricted. agy has no flag for MCP, no
flag for subagents, and no way to seed history — so omni lets it load whatever
it wants and carries prior conversation in as one flattened message, folded
into the first thing the user says so it costs no extra turn.

The one flag trap worth knowing: ``-p`` takes a value, so it must be passed as
``-p ""`` or it silently eats the next argument.
"""

from .account import Account
from .runner import Runner

__all__ = ["Account", "Runner"]
