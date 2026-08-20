"""A JSON-RPC client for ``codex app-server``.

One JSON object per line in both directions — no framing headers. Responses are
matched to requests by id; everything else is a notification and goes straight
to the listener. The server occasionally asks *us* something; anything omni does
not handle is declined politely so the server never waits on a dead end.
"""

import itertools
import json
import threading

from ...shared.proc import LineProcess

CLIENT = {"name": "silicon-omni", "version": "0.1.0"}


class AppServerError(RuntimeError):
    pass


class AppServer:
    def __init__(self, env=None, cwd=None, on_notify=None, argv=(), on_exit=None):
        self.argv = ["codex", "app-server", "--stdio", *argv]
        self.env = env
        self.cwd = cwd
        self.on_notify = on_notify
        self.on_exit = on_exit
        self.ids = itertools.count(1)
        self.pending: dict[int, list] = {}
        self.guard = threading.Lock()
        self.proc: LineProcess | None = None

    # ------------------------------------------------------------- transport

    def start(self, capabilities: dict | None = None) -> dict:
        self.proc = LineProcess(
            self.argv, on_line=self.line, env=self.env, cwd=self.cwd, on_exit=self.closed
        ).start()
        return self.call(
            "initialize", {"clientInfo": CLIENT, "capabilities": capabilities or {}}, timeout=30
        )

    def closed(self, code: int) -> None:
        """The server is gone: fail everything waiting on it rather than hang."""
        with self.guard:
            waiting, self.pending = self.pending, {}
        for slot in waiting.values():
            slot[0] = {"error": {"message": f"codex app-server exited with {code}"}}
            slot[1].set()
        if self.on_exit:
            self.on_exit(code)

    def line(self, text: str) -> None:
        try:
            message = json.loads(text)
        except json.JSONDecodeError:
            return
        if "method" in message and "id" in message:
            self.decline(message)
        elif "method" in message:
            if self.on_notify:
                self.on_notify(message["method"], message.get("params") or {})
        elif "id" in message:
            self.settle(message)

    def decline(self, message: dict) -> None:
        """A server-to-client request omni has no answer for."""
        self.write(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "error": {"code": -32601, "message": f"{message['method']} not handled by omni"},
            }
        )

    def settle(self, message: dict) -> None:
        with self.guard:
            slot = self.pending.pop(message["id"], None)
        if slot:
            slot[0] = message
            slot[1].set()

    def write(self, message: dict) -> bool:
        return bool(self.proc and self.proc.send_line(json.dumps(message)))

    # --------------------------------------------------------------- calling

    def call(self, method: str, params=None, timeout: float = 120.0):
        request_id = next(self.ids)
        slot = [None, threading.Event()]
        with self.guard:
            self.pending[request_id] = slot
        body = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            body["params"] = params
        if not self.write(body):
            raise AppServerError(f"codex app-server is not running (wanted {method})")
        if not slot[1].wait(timeout):
            with self.guard:
                self.pending.pop(request_id, None)
            raise AppServerError(f"{method} timed out after {timeout}s")
        message = slot[0]
        if "error" in message:
            raise AppServerError(f"{method}: {message['error'].get('message', message['error'])}")
        return message.get("result")

    def try_call(self, method: str, params=None, timeout: float = 15.0):
        """For calls that are allowed to fail — interrupts, best-effort probes."""
        try:
            return self.call(method, params, timeout)
        except AppServerError:
            return None

    def notify(self, method: str, params=None) -> None:
        body = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            body["params"] = params
        self.write(body)

    def stop(self) -> None:
        if self.proc:
            self.proc.stop(timeout=8)

    @property
    def alive(self) -> bool:
        return bool(self.proc and self.proc.alive)
