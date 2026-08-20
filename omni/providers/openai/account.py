"""Is Codex here, are we signed in, and how much is left.

Every answer lives behind the app-server protocol, so each question opens a
short-lived connection against the real ``~/.codex`` and closes it again. Login
is the exception: the server runs the browser callback itself, so that
connection is held open until the login lands.
"""

import threading

from ..base import Account as Base
from .appserver import AppServer, AppServerError

MINUTES = {300: "5h", 10080: "7d"}
BLANK = {"used": None, "reset": None}


def connect(on_notify=None) -> AppServer:
    server = AppServer(on_notify=on_notify)
    server.start()
    return server


def windows(payload: dict) -> dict:
    """Codex reports buckets by duration; omni reports 5h and 7d.

    Never trust ``primary`` to be the short window — on some plans it is the
    weekly one. Branch on ``windowDurationMins``, always.
    """
    out = {"5h": dict(BLANK), "7d": dict(BLANK)}
    buckets = [payload.get("rateLimits") or {}]
    buckets += list((payload.get("rateLimitsByLimitId") or {}).values())
    for bucket in buckets:
        for slot in ("primary", "secondary"):
            window = bucket.get(slot)
            if not isinstance(window, dict):
                continue
            key = MINUTES.get(window.get("windowDurationMins"))
            if key and out[key]["used"] is None:
                out[key] = {
                    "used": round(float(window.get("usedPercent", 0)) / 100, 4),
                    "reset": window.get("resetsAt"),
                }
    return out


class Account(Base):
    name = "openai"
    cli = "codex"

    def __init__(self):
        super().__init__()
        self.login_server: AppServer | None = None
        self.login_id = ""
        self.landed = threading.Event()
        self.outcome: dict = {}

    def probe(self) -> str:
        try:
            server = connect()
        except (AppServerError, OSError):
            return "unauthenticated"
        try:
            account = (server.call("account/read", {}, timeout=30) or {}).get("account")
        except AppServerError:
            account = None
        finally:
            server.stop()
        return "authenticated" if account else "unauthenticated"

    def start_auth(self) -> str:
        """Ask Codex to begin a ChatGPT login and hand back the URL to open."""
        self.landed.clear()
        self.login_server = connect(on_notify=self.watch)
        try:
            begun = self.login_server.call("account/login/start", {"type": "chatgpt"}, timeout=60)
        except AppServerError as exc:
            self.login_server.stop()
            return f"codex could not start a login: {exc}"
        self.login_id = begun.get("loginId", "")
        return begun.get("authUrl") or begun.get("verificationUrl") or str(begun)

    def watch(self, method: str, params: dict) -> None:
        if method == "account/login/completed" and params.get("loginId") == self.login_id:
            self.outcome = params
            self.landed.set()

    def finish_auth(self, code: str = "", timeout: float = 300.0) -> str:
        """Codex runs the callback itself, so this waits rather than types.

        ``code`` is accepted for symmetry with the other providers and ignored.
        """
        self.forget()
        if self.login_server:
            self.landed.wait(timeout)
            self.login_server.stop()
            self.login_server = None
        return self.auth_status

    @property
    def limits(self):
        try:
            server = connect()
        except (AppServerError, OSError):
            return "unauthenticated"
        try:
            payload = server.call("account/rateLimits/read", None, timeout=30)
        except AppServerError:
            return "unauthenticated"
        finally:
            server.stop()
        return windows(payload or {})
