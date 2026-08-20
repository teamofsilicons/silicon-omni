"""Is Claude Code here, are we signed in, and how much is left.

``claude auth status`` answers the first two as JSON. Usage comes from the
``get_usage`` control request, which is free and needs no credentials of our
own — Claude Code asks on its own behalf and hands the answer back. That is
much better than reading somebody's keychain, and it keeps working when the
storage moves.
"""

import json
import subprocess

from ..base import Account as Base
from ..login import Login
from . import control

WINDOWS = {"five_hour": "5h", "seven_day": "7d"}


def run(argv: list[str], timeout: float = 20.0) -> tuple[int, str]:
    try:
        done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired):
        return 1, ""
    return done.returncode, (done.stdout or done.stderr or "").strip()


def window(entry) -> dict:
    """``{"utilization": 0-100, "resets_at": iso}`` becomes omni's ``{"used", "reset"}``."""
    if not isinstance(entry, dict):
        return {"used": None, "reset": None}
    used = entry.get("utilization")
    return {"used": None if used is None else round(float(used) / 100, 4), "reset": entry.get("resets_at")}


class Account(Base):
    name = "claude"
    cli = "claude"

    def probe(self) -> str:
        code, out = run(["claude", "auth", "status"])
        if code:
            return "unauthenticated"
        try:
            return "authenticated" if json.loads(out).get("loggedIn") else "unauthenticated"
        except json.JSONDecodeError:
            return "unauthenticated"

    def start_auth(self) -> str:
        """Start ``claude auth login`` and hand back the URL it wants opened."""
        self.login = Login(["claude", "auth", "login"])
        return self.login.start()

    def finish_auth(self, code: str = "") -> str:
        """Give the code (or redirect URL) back to the login, then re-check."""
        self.forget()
        if getattr(self, "login", None):
            self.login.finish(code)
        return self.auth_status

    @property
    def limits(self):
        if self.auth_status != "authenticated":
            return "unauthenticated"
        usage = control.ask("get_usage") or {}
        buckets = usage.get("rate_limits") or {}
        return {short: window(buckets.get(long)) for long, short in WINDOWS.items()}
