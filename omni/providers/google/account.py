"""Is agy here, are we signed in, and how much is left.

``agy models`` is the safe auth probe: exit 0 means signed in. Never probe with
``-p`` — unauthenticated print mode starts an interactive login and blocks for
a minute, spraying non-JSON over stdout.

Quota comes from ``agy -p "/usage"``, which the CLI answers itself: no model is
called and no quota is spent.
"""

import json
import subprocess

from ...shared import clock
from ..base import Account as Base
from ..login import Login

USAGE = ["agy", "-p", "/usage", "--output-format", "stream-json"]
WINDOWS = {"5h": "5h", "weekly": "7d"}
BLANK = {"used": None, "reset": None}


def buckets(lines: str) -> list[dict]:
    for line in lines.splitlines():
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            continue
        if data.get("event") != "command_result":
            continue
        groups = ((data.get("command") or {}).get("data") or {}).get("groups") or []
        return [bucket for group in groups for bucket in group.get("buckets") or []]
    return []


def windows(lines: str) -> dict:
    """agy reports remaining, per model group. omni reports used, worst group first."""
    out = {"5h": dict(BLANK), "7d": dict(BLANK)}
    for bucket in buckets(lines):
        key = WINDOWS.get(bucket.get("window"))
        if not key:
            continue
        used = round(1 - float(bucket.get("remaining_fraction", 1)), 4)
        if out[key]["used"] is None or used > out[key]["used"]:
            out[key] = {"used": used, "reset": clock.iso(bucket.get("reset_time"))}
    return out


class Account(Base):
    name = "google"
    cli = "agy"

    def probe(self) -> str:
        try:
            done = subprocess.run(["agy", "models"], capture_output=True, text=True, timeout=60)
        except (OSError, subprocess.TimeoutExpired):
            return "unauthenticated"
        return "authenticated" if done.returncode == 0 else "unauthenticated"

    def start_auth(self) -> str:
        """agy only offers a login as a side effect of running something.

        ``/usage`` is the cheapest thing to run: the CLI answers it itself, so
        the login is the only thing that actually happens.
        """
        self.login = Login(USAGE)
        return self.login.start(timeout=60)

    def finish_auth(self, code: str = "") -> str:
        self.forget()
        if getattr(self, "login", None):
            self.login.finish(code, timeout=90)
        return self.auth_status

    @property
    def limits(self):
        if self.auth_status != "authenticated":
            return "unauthenticated"
        try:
            done = subprocess.run(USAGE, capture_output=True, text=True, timeout=90)
        except (OSError, subprocess.TimeoutExpired):
            return {"5h": dict(BLANK), "7d": dict(BLANK)}
        return windows(done.stdout)
