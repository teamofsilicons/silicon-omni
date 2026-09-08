"""The providers omni speaks.

The adapters themselves live in the daemon — this is only the part of them a
Python program touches: which ones are usable, and the test double.
"""

from ..client import call
from . import test

#: The providers built into omni. A test double registered by
#: :func:`omni.providers.test.install` appears alongside these.
BUILT_IN = ("claude-code-cli", "codex-app-server", "antigravity-cli")


def available(limit_to=None) -> list[str]:
    """Installed *and* logged in — the only providers omni will route to."""
    return call("providers", providers=list(limit_to) if limit_to else None, timeout=180.0)


def account(name: str):
    """The account handle for a provider — auth, limits, install state."""
    from ..inference import ProviderHandle

    return ProviderHandle(name)


__all__ = ["available", "account", "test", "BUILT_IN"]
