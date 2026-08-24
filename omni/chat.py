"""One omni session, as seen from Python.

The conversation itself lives in the daemon: it owns the providers, the history
and the turn boundaries, and it keeps running whether or not this process is
attached. What is here is the part that has to be in Python — your callbacks,
and the calls that reach the session.

That is the whole change from omni 0.3: this file used to *be* the engine, and
now it talks to one. Nothing you write against it needs to know that. The rule
the design hangs off is still the daemon's rule: **nothing changes mid turn**.

Because the session outlives the process, two things are now true that were not
before. Several programs can hold the same session at once — each gets every
event, and any of them can send. And leaving is cheap: close your program and
come back, and the provider is still warm.
"""

import os
import queue
import threading
from typing import Callable, Sequence

from .client import DaemonError, Link
from .events import Event

IDLE = "idle"
WAITING = "waiting"
BUSY = "busy"
STOPPED = "stopped"


class Bus:
    """Callback fan-out.

    omni is event driven: everything interesting is handed to whoever
    subscribed. A handler that raises must never take the session down with it,
    so failures are routed to ``on_error`` instead of propagating.
    """

    def __init__(self, on_error: Callable | None = None):
        self.handlers: list[Callable] = []
        self.on_error = on_error

    def subscribe(self, fn: Callable) -> Callable:
        """Register a handler. Returns it unchanged, so it works as a decorator."""
        self.handlers.append(fn)
        return fn

    def emit(self, payload) -> None:
        for fn in list(self.handlers):
            try:
                fn(payload)
            except Exception as exc:  # a subscriber's bug is not the run's problem
                if self.on_error:
                    self.on_error(exc, fn, payload)


class Chat:
    """A persistent conversation. Get one from ``Inference.load_or_create_session``."""

    def __init__(self, session_id: str, providers: Sequence[str]):
        self.session_id = session_id
        self.providers = list(providers)
        # Its own connection, opened when it is first needed. The daemon tells
        # sessions apart by connection, so sharing one would mean two chats on
        # the same id could not be told apart — or detached separately.
        self.connection: Link | None = None
        self.events = Bus(on_error=self.handler_failed)
        self.log = Bus(on_error=self.log_failed)
        # Callbacks run on this session's own thread, in order, one at a time —
        # so a slow handler holds up its own session and nobody else's.
        self.inbox: queue.Queue = queue.Queue()
        self.caller: threading.Thread | None = None
        self.pending: list[tuple[str, object]] = []
        self.opened = False
        self._started = False
        self.finished = False
        self._start_lock = threading.RLock()
        self.state = {"status": IDLE, "seq": -1, "queued": 0, "in_turn": False}
        self.seen = -1

    @property
    def link(self) -> Link:
        if self.connection is None or not self.connection.alive:
            self.connection = Link.open()
        return self.connection

    # ------------------------------------------------------------------ setup

    def active_inference_providers(self, providers: Sequence[str]) -> None:
        """Limit which providers this chat may use. Applied at the next turn boundary."""
        self.providers = list(providers)
        self.change("providers", self.providers)

    def intelligence(self, level: int) -> None:
        """0-10 across every active provider. May change model *and* provider."""
        self.change("level", int(level))

    #: the spelling used in the README's example
    inteligence = intelligence

    def system_prompt(self, text: str) -> None:
        """Replace the provider's own session prompt."""
        self.change("system_prompt", text)

    def system_prompt_file(self, path: str) -> None:
        self.system_prompt(open(path, encoding="utf-8").read())

    def append_system_prompt(self, text: str) -> None:
        """Keep the provider's prompt and add to it."""
        self.change("append_system_prompt", text)

    def append_system_prompt_file(self, path: str) -> None:
        self.append_system_prompt(open(path, encoding="utf-8").read())

    def disable_subagents(self) -> None:
        """No provider-side subagents, so only the workers you define get used.

        Already the default; here so asking for it out loud still reads.
        """
        self.change("subagents", False)

    def enable_subagents(self) -> None:
        """Let the provider spawn its own subagents. Off unless you ask."""
        self.change("subagents", True)

    def disable_mcp(self) -> None:
        """No MCP servers, no external connectors. Already the default."""
        self.change("mcp", False)

    def enable_mcp(self) -> None:
        """Let the provider load its MCP servers and connectors. Off unless you ask.

        Not every provider can honour it — codex is always jailed and agy has no
        switch at all — and the one that cannot says so.
        """
        self.change("mcp", True)

    def disable_autoremoving_unauthenticated_providers(self) -> None:
        """Stop dropping a provider that loses its login mid-run.

        On by default: an unauthenticated CLI cannot finish the turn, so omni
        takes it off this chat's list and resolves the same intelligence level
        again over whoever is left. Turn it off and the auth error is reported
        and the turn simply ends.
        """
        self.change("autoremove", False)

    def cwd(self, path: str) -> None:
        """Where the provider runs its tools.

        Pinned to the session the first time, because Claude resumes by working
        directory. Moving it mid-session ports the conversation to the new one.
        """
        self.change("cwd", os.path.realpath(path))

    def change(self, what: str, value) -> None:
        """Ask for a setting. Held until ``start`` if the session is not open."""
        if not self.opened:
            self.pending.append((what, value))
            return
        self.link.call("set", session=self.session_id, what=what, value=value)

    # ------------------------------------------------------------- callbacks

    def on_event(self, fn: Callable[[Event], None]) -> Callable:
        """Decorator. Every event, as it happens."""
        return self.events.subscribe(fn)

    def logs(self, fn: Callable[[Event], None]) -> Callable:
        """Decorator. Everything ``on_event`` sees, plus omni's own bookkeeping."""
        return self.log.subscribe(fn)

    def handler_failed(self, exc, fn, event) -> None:
        self.log.emit(self.blame(exc, fn))

    def log_failed(self, exc, fn, event) -> None:
        """A broken log handler cannot be reported to the log handlers."""
        print(f"omni: log handler {getattr(fn, '__name__', fn)} raised {exc!r}")

    def blame(self, exc, fn) -> Event:
        return Event(
            type=Event.ERROR,
            kind="handler",
            ok=False,
            session=self.session_id,
            error=f"{getattr(fn, '__name__', fn)}: {exc!r}",
        )

    # ------------------------------------------------------------- lifecycle

    @property
    def status(self) -> str:
        """``idle`` before start, then ``busy`` / ``waiting``, then ``stopped``."""
        return self.state.get("status", IDLE)

    @property
    def idle(self) -> bool:
        """Waiting, with no turn open. What a polling loop should check."""
        return self.status == WAITING and not self.state.get("in_turn") and self.inbox.empty()

    @property
    def provider(self) -> str:
        """Which provider is running this conversation right now."""
        return self.state.get("provider", "")

    @property
    def model(self) -> str:
        return self.state.get("model", "")

    def start(self, since: int = 0) -> "Chat":
        """Open the session and start hearing about it.

        Everything asked for before this — providers, intelligence, prompts —
        is applied here, in the order it was asked for.

        ``since`` is the first ``event.seq`` to replay. The default is the whole
        conversation, so a program that reconnects to a session sees everything
        that happened while it was away. Pass ``-1`` to hear only what happens
        from now on, or the seq after the last one you handled to pick up
        exactly where you left off.
        """
        with self._start_lock:
            if self.finished:
                raise RuntimeError(
                    f"session {self.session_id!r} is stopped; load it again to continue"
                )
            if self.opened:
                return self

            # Let replay frames collect in the inbox while ``open`` is in
            # flight, then start callbacks only after the request succeeded.
            # Besides making failed starts tidy, this means a replay callback
            # can safely call back into ``start``/``send``: the chat is already
            # open by the time user code runs.
            connection = self.link
            connection.listen(self.session_id, self.arrived)
            try:
                result = connection.call(
                    "open",
                    session=self.session_id,
                    providers=self.providers,
                    value=[{"what": what, "value": value} for what, value in self.pending],
                    **{"from": int(since)},
                )
                self.state = result.get("snapshot") or self.state
            except BaseException:
                # ``open`` may have reached the daemon even if its reply did
                # not reach us. Closing this connection makes the daemon drop
                # any subscription it created, so a retry cannot hear every
                # event twice.
                connection.unlisten(self.session_id)
                connection.close()
                if self.connection is connection:
                    self.connection = None
                raise

            self.opened = True
            self._started = True
            self.pending.clear()
            self.seen = max(self.seen, int(self.state.get("seq", -1)))
            self.dispatching()
            return self

    def send(self, text: str) -> None:
        """Send a message. Opens a turn, or lands inside the one already running.

        Returns immediately; ``status`` flips to ``busy`` before it does, so a
        caller polling in a loop never sees a false lull.
        """
        if self.finished:
            raise RuntimeError(f"session {self.session_id!r} is stopped; load it again to continue")
        if not self.opened:
            # A detach or daemon restart resumes where this object left off.
            # An explicit ``start()`` still honours its documented default and
            # replays the whole conversation.
            self.start(since=self.seen + 1)
        self.state["status"] = BUSY
        self.link.call("send", session=self.session_id, text=text)

    def stop(self) -> None:
        """End the chat: the providers are shut down and the session is closed.

        This is an instruction, not a disconnect. To leave a session running —
        so the next program to open it finds the provider already warm — use
        :meth:`detach` instead, or simply exit.
        """
        if self.finished:
            return
        self.finished = True
        if self._started:
            try:
                self.link.call("stop", session=self.session_id)
            except DaemonError:
                pass
        self.settle_down()
        self.state["status"] = STOPPED
        if self.connection is not None:
            self.connection.close()
            self.connection = None

    def detach(self) -> None:
        """Stop listening, and leave the session running in the daemon.

        The conversation stays warm: its provider is still up, and opening the
        same id again — from here or from another program — costs nothing.
        """
        if self.opened and not self.finished:
            try:
                self.link.call("detach", session=self.session_id)
            except DaemonError:
                pass
        self.opened = False
        self.settle_down()

    def refresh(self) -> dict:
        """Ask the daemon where the session actually is, rather than trusting
        the last thing it told us. Rarely needed; useful after a reconnect."""
        result = self.link.call("status", session=self.session_id)
        self.state = result.get("snapshot") or self.state
        return self.state

    def history(self, since: int = 0) -> list[Event]:
        """Every event this session has ever recorded, from ``since`` on.

        Read straight out of the log, so it works whether or not the session is
        open, and whoever wrote it.
        """
        result = self.link.call("events", session=self.session_id, **{"from": since})
        return [Event.from_dict(item) for item in result.get("events", [])]

    def __enter__(self) -> "Chat":
        return self.start()

    def __exit__(self, *exc) -> None:
        self.stop()

    # ----------------------------------------------------------- the stream

    def arrived(self, frame: dict) -> None:
        """A line from the daemon about this session. Called on the link thread."""
        self.inbox.put(frame)

    def dispatching(self) -> None:
        if self.caller and self.caller.is_alive():
            return
        self.caller = threading.Thread(
            target=self.deliver, daemon=True, name=f"omni:{self.session_id}"
        )
        self.caller.start()

    def deliver(self) -> None:
        """Hand events to callbacks, one at a time, in the order they happened."""
        current = threading.current_thread()
        try:
            while True:
                frame = self.inbox.get()
                if frame is None:
                    return
                if frame.get("stream") == "disconnected":
                    # A dead daemon is not a stopped conversation. Its log is
                    # still on disk, and the next send can start a daemon and
                    # reopen exactly after the last event this client saw.
                    self.opened = False
                    self.state["status"] = WAITING
                    self.state["in_turn"] = False
                    return
                if frame.get("stream") == "gone":
                    self.state = frame.get("snapshot") or self.state
                    self.state["status"] = STOPPED
                    self.opened = False
                    self.finished = True
                    return
                # The daemon says where the session stands as each event goes
                # out, so nothing here has to work it out a second time.
                self.state = frame.get("snapshot") or self.state
                event = Event.from_dict(frame.get("event") or {})
                if event.seq > self.seen:
                    self.seen = event.seq
                self.events.emit(event)
                self.log.emit(event)
        finally:
            # Do not let a just-finished dispatcher prevent a reconnect from
            # starting its replacement.
            if self.caller is current:
                self.caller = None


    def settle_down(self) -> None:
        # Lifecycle methods must not start a daemon merely in order to leave
        # it. In particular, ``stop`` and ``detach`` are harmless on a Chat
        # whose first ``start`` failed.
        if self.connection is not None:
            self.connection.unlisten(self.session_id)
        if self.caller and self.caller.is_alive():
            self.inbox.put(None)
            if self.caller is not threading.current_thread():
                self.caller.join(timeout=5)
        self.caller = None

    def __repr__(self) -> str:
        return f"<Chat {self.session_id} {self.status} provider={self.provider!r}>"
