"""Connections to the daemon.

Each open ``Chat`` owns a socket because the connection is its subscription. A
separate process-wide socket is shared by one-off provider, account, quota, and
dial calls. Every connection has a reader thread that sorts its lines: one with
an ``id`` answers a request, while one with a ``stream`` belongs to the session
opened on that connection.

Requests are answered out of order, and events arrive in between them, so
nothing here ever assumes the next line is the one it wanted.
"""

import itertools
import json
import socket
import threading

from .daemon import DaemonError, start

PROTOCOL = 1


class Link:
    """A connection to the daemon.

    Two kinds of caller want different things. A one-off question — which
    providers are logged in, what does intelligence 7 mean — wants
    :meth:`shared`, the connection this process keeps for asking things. A
    session wants :meth:`open`: its own connection, because *the connection is
    the subscription*. Two sessions sharing one would be two subscriptions
    the daemon cannot tell apart, and detaching one would detach both.
    """

    _shared: "Link | None" = None
    _guard = threading.Lock()

    def __init__(self):
        self.path = start()
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(str(self.path))
        self.file = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.ids = itertools.count(1)
        self.waiting: dict[int, list] = {}
        self.listeners: dict[str, callable] = {}
        self.lock = threading.Lock()
        # Writing and bookkeeping are separate locks: a write that blocks must
        # not stop the reader thread from settling what it already has.
        self.pen = threading.Lock()
        self.alive = True
        self.reader = threading.Thread(target=self.pump, daemon=True, name="omni:link")
        self.reader.start()
        try:
            hello = self.call("ping", timeout=5.0)
        except BaseException:
            self.close()
            raise
        if hello.get("protocol") != PROTOCOL:
            self.close()
            raise DaemonError(
                f"omni protocol mismatch: Python speaks {PROTOCOL}, "
                f"but the running daemon speaks {hello.get('protocol')!r}. "
                "Stop the old daemon and retry."
            )
        self.hello = hello

    # ------------------------------------------------------------- lifecycle

    @classmethod
    def open(cls) -> "Link":
        """A connection of one's own. What a session holds."""
        return cls()

    @classmethod
    def shared(cls) -> "Link":
        """The one connection for this process, opened on first use."""
        with cls._guard:
            if cls._shared is None or not cls._shared.alive:
                cls._shared = Link()
            return cls._shared

    @classmethod
    def forget(cls) -> None:
        """Drop the shared connection; the next caller opens a new one."""
        with cls._guard:
            if cls._shared is not None:
                cls._shared.close()
            cls._shared = None

    def close(self) -> None:
        self.alive = False
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()

    # ----------------------------------------------------------------- lines

    def pump(self) -> None:
        """Read forever, sorting replies from events."""
        try:
            for line in self.file:
                line = line.strip()
                if not line:
                    continue
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "stream" in message:
                    self.deliver(message)
                else:
                    self.settle(message)
        except (OSError, ValueError):
            pass
        finally:
            self.hang_up()

    def deliver(self, frame: dict) -> None:
        handler = self.listeners.get(frame.get("session", ""))
        if handler:
            handler(frame)

    def settle(self, reply: dict) -> None:
        with self.lock:
            slot = self.waiting.pop(reply.get("id", -1), None)
        if slot:
            slot[0] = reply
            slot[1].set()

    def hang_up(self) -> None:
        """The daemon went away: fail everything waiting rather than hang."""
        self.alive = False
        with self.lock:
            waiting, self.waiting = self.waiting, {}
        for slot in waiting.values():
            slot[0] = {"ok": False, "error": "the omni daemon went away"}
            slot[1].set()
        for session, handler in list(self.listeners.items()):
            handler(
                {
                    "stream": "disconnected",
                    "session": session,
                    "reason": "daemon went away",
                }
            )
        self.listeners.clear()

    # --------------------------------------------------------------- calling

    def call(self, op: str, timeout: float = 120.0, **fields):
        """Ask the daemon one thing. Raises :class:`DaemonError` if it refuses."""
        request_id = next(self.ids)
        slot = [None, threading.Event()]
        with self.lock:
            self.waiting[request_id] = slot
        body = json.dumps({"id": request_id, "op": op, **fields})
        try:
            with self.pen:
                self.file.write(body + "\n")
                self.file.flush()
        except (OSError, ValueError) as exc:
            with self.lock:
                self.waiting.pop(request_id, None)
            raise DaemonError(f"could not send {op}: {exc}") from exc
        if not slot[1].wait(timeout):
            with self.lock:
                self.waiting.pop(request_id, None)
            raise DaemonError(f"{op} got no answer in {timeout:g}s")
        reply = slot[0]
        if not reply.get("ok"):
            raise DaemonError(reply.get("error") or f"{op} failed")
        return reply.get("result")

    # ------------------------------------------------------------- listening

    def listen(self, session: str, handler) -> None:
        self.listeners[session] = handler

    def unlisten(self, session: str) -> None:
        self.listeners.pop(session, None)


def call(op: str, **fields):
    """One-off request on the shared connection."""
    return Link.shared().call(op, **fields)
