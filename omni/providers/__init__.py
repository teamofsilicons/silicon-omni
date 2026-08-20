"""The providers omni speaks, and the lookup everything else goes through.

Adapters are imported lazily: ``import omni`` should not drag in three CLI
protocols you may not use. Third parties (and tests) can add their own with
:func:`register`.
"""

import importlib

from .base import Account, Config, Runner

#: provider name -> module exposing ``Account`` and ``Runner``
MODULES = {
    "claude": "omni.providers.claude",
    "google": "omni.providers.google",
    "openai": "omni.providers.openai",
}

EXTRA: dict[str, tuple] = {}
CACHE: dict[str, Account] = {}


def register(name: str, account_cls, runner_cls) -> None:
    """Add a provider at runtime."""
    EXTRA[name] = (account_cls, runner_cls)
    CACHE.pop(name, None)


def names() -> list[str]:
    return list(MODULES) + [n for n in EXTRA if n not in MODULES]


def classes(name: str) -> tuple:
    if name in EXTRA:
        return EXTRA[name]
    if name not in MODULES:
        raise LookupError(f"unknown provider {name!r}; known: {names()}")
    module = importlib.import_module(MODULES[name])
    return module.Account, module.Runner


def account(name: str) -> Account:
    """The shared account handle for a provider — auth, limits, install state."""
    if name not in CACHE:
        CACHE[name] = classes(name)[0]()
    return CACHE[name]


def runner_for(name: str):
    return classes(name)[1]


def available(limit_to=None) -> list[str]:
    """Installed *and* logged in — the only providers omni will route to."""
    allowed = set(limit_to) if limit_to else None
    return [n for n in names() if (allowed is None or n in allowed) and account(n).available]


__all__ = ["Account", "Runner", "Config", "account", "runner_for", "available", "register", "names"]
