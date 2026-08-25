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
import warnings
from typing import Callable, Sequence

from .client import DaemonError, Link
from .events import Event

IDLE = "idle"
WAITING = "waiting"
BUSY = "busy"
STOPPED = "stopped"


def _normalize_snapshot(snapshot: dict) -> dict:
    """The public Python spelling of a daemon session snapshot.

    0.7 replaced the single ``level`` setting with an ``ask``, which is a key,
    a number, or a model by name. A daemon old enough to send ``level`` is one
    this client cannot talk to anyway, so the old key is dropped rather than
    translated into a shape it does not fit.
    """
    normalized = dict(snapshot)
    normalized.pop("level", None)
    return normalized


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


class _LinkGeneration:
    """One attempt to subscribe this Chat to one daemon connection.

    A Link can finish announcing its disconnect after its replacement has
    already opened. Frames carry this identity through the callback queue so
    that late work from the old link cannot mutate the replacement's state.
    """

    def __init__(self):
        self.ready = threading.Event()
        self.accepted = False


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
        self.inbox: queue.Queue[tuple[_LinkGeneration, dict] | None] = queue.Queue()
        self.caller: threading.Thread | None = None
        self.pending: list[tuple[str, object]] = []
        self.opened = False
        self._started = False
        self.finished = False
        self._start_lock = threading.RLock()
        self._dispatch_lock = threading.Lock()
        self._state_lock = threading.Lock()
        self._generation: _LinkGeneration | None = None
        self.state = {"status": IDLE, "seq": -1, "queued": 0, "in_turn": False}
        self.seen = -1
        # A daemon can stream an older CONFIG frame while the synchronous
        # `send` request is in flight. Keep the accepted message locally busy
        # until its START/INJECTED record arrives, so polling can never observe
        # that stale frame as a false idle boundary.
        self._awaiting: list[dict] = []

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

    def model(
        self,
        key: str | None = None,
        *,
        intelligence: int | None = None,
        bench: str | None = None,
        model: str | None = None,
        effort: str = "",
        fast: bool = False,
        provider: str | None = None,
    ) -> None:
        """Say what should answer. May change model, effort *and* provider.

        Three ways, and exactly one of them per call:

        ``chat.model("code")``
            A shortlist somebody curated. The first vendor on it you are
            signed into answers, so losing one walks you a place down the list.

        ``chat.model(intelligence=7)``
            The 0-10 dial: the left edge of a board, where a model earns a rung
            when nothing else is both better *and* cheaper. ``bench`` picks the
            board.

        ``chat.model(model="gemini-3.7-flash-low", provider="google")``
            You already know. The name goes to the CLI verbatim, so a model
            released this morning works without omni knowing about it.
            ``fast`` asks for the CLI's faster tier where it has one, and is
            ignored where it does not.
        """
        said = [name for name, given in
                (("key", key is not None),
                 ("intelligence", intelligence is not None),
                 ("model", model is not None)) if given]
        if len(said) != 1:
            raise ValueError(
                "say exactly one of key, intelligence or model"
                + (f"; got {', '.join(said)}" if said else "")
            )

        if key is not None:
            ask: dict[str, object] = {"how": "key", "key": str(key).strip().lower()}
        elif intelligence is not None:
            ask = {"how": "intelligence", "value": int(intelligence)}
            if bench:
                ask["bench"] = str(bench).strip()
        else:
            ask = {"how": "model", "model": str(model).strip(), "effort": effort, "fast": bool(fast)}
            if provider:
                ask["provider"] = str(provider).strip()
        self.change("model", ask)

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
        takes it off this chat's list and resolves the same ask
        again over whoever is left. Turn it off and the auth error is reported
        and the turn simply ends.
        """
        self.change("autoremove", False)

    def enable_autoremoving_unauthenticated_providers(self) -> None:
        """Drop an unauthenticated provider and continue elsewhere.

        This is the default. The explicit method pairs with
        :meth:`disable_autoremoving_unauthenticated_providers` so callers can
        set the policy from a boolean without relying on defaults.
        """
        self.change("autoremove", True)

    def cwd(self, path: str) -> None:
        """Where the provider runs its tools.

        Pinned to the session the first time, because Claude resumes by working
        directory. Moving it mid-session ports the conversation to the new one.
        """
        self.change("cwd", os.path.realpath(path))

    def change(self, what: str, value) -> None:
        """Ask for a setting. Held until ``start`` if the session is not open."""
        if not self.opened or self.connection is None or not self.connection.alive:
            self.opened = False
            self.pending.append((what, value))
            return
        self.connection.call("set", session=self.session_id, what=what, value=value)

    # ------------------------------------------------------------- callbacks

    def on_event(self, fn: Callable[[Event], None]) -> Callable:
        """Decorator. Every daemon event this client receives, in order.

        That includes requested replay and bookkeeping events such as
        ``CONFIG``. Each one is also a durable member of :meth:`history`.
        """
        return self.events.subscribe(fn)

    def logs(self, fn: Callable[[Event], None]) -> Callable:
        """Decorator. Every daemon event, plus local callback failures.

        A local ``ERROR``/``handler`` record has no sequence number and is not
        persisted, because it describes this Python process rather than the
        shared conversation. There is no second daemon log schema.
        """
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
        with self._state_lock:
            if self._awaiting:
                return BUSY
            return self.state.get("status", IDLE)

    @property
    def idle(self) -> bool:
        """Waiting, with no turn open. What a polling loop should check."""
        with self._state_lock:
            settled = (
                not self._awaiting
                and self.state.get("status") == WAITING
                and not self.state.get("in_turn")
            )
        return settled and self.inbox.empty()

    @property
    def provider(self) -> str:
        """Which provider is running this conversation right now."""
        return self.state.get("provider", "")

    @property
    def running_model(self) -> str:
        """The model actually up right now.

        Not always what was asked for: a key resolves to different models as
        providers come and go. :meth:`model` is the setter.
        """
        return self.state.get("model", "")

    @property
    def effort(self) -> str:
        """The current provider's effort setting, if it reports one."""
        return self.state.get("effort", "")

    @property
    def current_ask(self) -> dict | None:
        """What this chat was last told to run, once the daemon has reported it.

        This is deliberately separate from :meth:`model`, which asks
        for a future setting change and remains callable.
        """
        ask = self.state.get("ask")
        return dict(ask) if isinstance(ask, dict) else None

    def start(self, since: int = 0) -> "Chat":
        """Open the session and start hearing about it.

        Everything asked for before this — providers, the model, prompts —
        is applied here, in the order it was asked for.

        ``since`` is the first ``event.seq`` to replay. The default is the whole
        conversation, so a program that reconnects to a session sees everything
        that happened while it was away. Pass ``-1`` to hear only what happens
        from now on, or the seq after the last one you handled to pick up
        exactly where you left off.
        """
        with self._start_lock:
            since = int(since)
            if self.finished:
                raise RuntimeError(
                    f"session {self.session_id!r} is stopped; load it again to continue"
                )
            if self.opened and self.connection is not None and self.connection.alive:
                return self
            self.opened = False

            # Let replay frames collect in the inbox while ``open`` is in
            # flight, then start callbacks only after the request succeeded.
            # Besides making failed starts tidy, this means a replay callback
            # can safely call back into ``start``/``send``: the chat is already
            # open by the time user code runs.
            connection = self.link
            generation = _LinkGeneration()
            self._generation = generation
            connection.listen(
                self.session_id,
                lambda frame, generation=generation: self.arrived(frame, generation),
            )
            try:
                result = connection.call(
                    "open",
                    session=self.session_id,
                    providers=self.providers,
                    cwd=os.getcwd(),
                    value=[{"what": what, "value": value} for what, value in self.pending],
                    **{"from": since},
                )
                with self._state_lock:
                    snapshot = result.get("snapshot")
                    self.state = _normalize_snapshot(snapshot) if snapshot else self.state
            except BaseException:
                # ``open`` may have reached the daemon even if its reply did
                # not reach us. Closing this connection makes the daemon drop
                # any subscription it created, so a retry cannot hear every
                # event twice.
                connection.unlisten(self.session_id)
                connection.close()
                if self.connection is connection:
                    self.connection = None
                # A dispatcher may already be waiting on a replay frame this
                # failed open received. Release it to discard that generation.
                generation.ready.set()
                raise

            self.opened = True
            self._started = True
            self.pending.clear()
            with self._state_lock:
                # `seen` is the callback replay cursor, not merely the latest
                # state the daemon reported. Positive replay ranges advance it
                # only as their frames are delivered. A negative range skips
                # history deliberately, so the snapshot becomes its baseline.
                floor = int(self.state.get("seq", -1)) if since < 0 else since - 1
                self.seen = max(self.seen, floor)
            generation.accepted = True
            generation.ready.set()
            self.dispatching()
            return self

    def send(self, text: str) -> None:
        """Send a message. Opens a turn, or lands inside the one already running.

        Waits for the daemon to accept or reject the request, but not for model
        output or the end of the turn. ``status`` flips to ``busy`` first, so a
        caller polling in a loop never sees a false lull.
        """
        if self.finished:
            raise RuntimeError(f"session {self.session_id!r} is stopped; load it again to continue")
        if not self.opened or self.connection is None or not self.connection.alive:
            self.opened = False
            # A detach or daemon restart resumes where this object left off.
            # An explicit ``start()`` still honours its documented default and
            # replays the whole conversation.
            self.start(since=self.seen + 1)
        with self._state_lock:
            ticket = {
                "text": text,
                "after": max(self.seen, int(self.state.get("seq", -1))),
            }
            self._awaiting.append(ticket)
            self.state["status"] = BUSY
        try:
            self.link.call("send", session=self.session_id, text=text)
        except BaseException:
            with self._state_lock:
                if ticket in self._awaiting:
                    self._awaiting.remove(ticket)
            raise

    def stop(self) -> None:
        """End the chat: the providers are shut down and the session is closed.

        This is an instruction, not a disconnect. To leave a session running —
        so the next program to open it finds the provider already warm — use
        :meth:`detach` instead, or simply exit.
        """
        if self.finished:
            return

        # A Chat that never opened has no remote lifecycle to acknowledge.
        if not self._started:
            self.finished = True
            self.opened = False
            self.settle_down()
            with self._state_lock:
                self._awaiting.clear()
                self.state["status"] = STOPPED
            if self.connection is not None:
                self.connection.close()
                self.connection = None
            return

        connection = self.connection
        if connection is None or not connection.alive:
            self._connection_lost(connection)
            raise DaemonError(
                f"cannot stop session {self.session_id!r}: the daemon connection is gone; "
                "load the session and retry"
            )
        try:
            connection.call("stop", session=self.session_id)
        except Exception:
            # A refusal is authoritative; a timeout is not. In both cases the
            # caller must hear the error, and closing the subscription prevents
            # an uncertain old link from continuing to mutate this object.
            self._connection_lost(connection)
            raise

        # Only the daemon's acknowledgement makes this object terminal.
        self.finished = True
        self.opened = False
        self.settle_down()
        with self._state_lock:
            self._awaiting.clear()
            self.state["status"] = STOPPED
        connection.close()
        if self.connection is connection:
            self.connection = None

    def detach(self) -> None:
        """Stop listening, and leave the session running in the daemon.

        The conversation stays warm: its provider is still up, and opening the
        same id again — from here or from another program — costs nothing.
        """
        if self.finished or not self.opened:
            return

        connection = self.connection
        if connection is None or not connection.alive:
            # A closed transport is already a definitive detach: the daemon
            # identifies listeners by connection and cannot retain this one.
            self._connection_lost(connection)
            return
        try:
            connection.call("detach", session=self.session_id)
        except Exception:
            # Even if the request reached the daemon and only its reply was
            # lost, closing the socket makes forgetting this listener certain.
            self._connection_lost(connection)
            raise

        # Do not claim detachment until the daemon has acknowledged it.
        self.opened = False
        self.settle_down()

    def _connection_lost(self, connection: Link | None) -> None:
        """Make an uncertain lifecycle call safely retryable, without hiding it."""
        self.opened = False
        if connection is not None:
            try:
                connection.unlisten(self.session_id)
            except Exception:
                pass
            try:
                connection.close()
            except Exception:
                pass
            if self.connection is connection:
                self.connection = None
        self.settle_down()
        with self._state_lock:
            self._awaiting.clear()
            self.state["status"] = WAITING
            self.state["in_turn"] = False

    def refresh(self) -> dict:
        """Ask the daemon where the session actually is, rather than trusting
        the last thing it told us. Rarely needed; useful after a reconnect."""
        if not self.opened or self.connection is None or not self.connection.alive:
            self.start(since=self.seen + 1)
        result = self.connection.call("status", session=self.session_id)
        with self._state_lock:
            snapshot = result.get("snapshot")
            self.state = _normalize_snapshot(snapshot) if snapshot else self.state
            return self.state.copy()

    def history(self, since: int = 0) -> list[Event]:
        """Every persisted daemon event from sequence ``since`` onward.

        Read straight from the shared session event log, so it works whether
        or not this client is attached. Python-local callback failures sent to
        :meth:`logs` are not part of that durable history.
        """
        result = self.link.call("events", session=self.session_id, **{"from": since})
        return [Event.from_dict(item) for item in result.get("events", [])]

    def __enter__(self) -> "Chat":
        return self.start()

    def __exit__(self, *exc) -> None:
        self.detach()

    # ----------------------------------------------------------- the stream

    def arrived(
        self, frame: dict, generation: _LinkGeneration | None = None
    ) -> None:
        """A line from the daemon about this session. Called on the link thread."""
        generation = generation or self._generation
        if generation is None:
            return
        self.inbox.put((generation, frame))
        # During open, frames intentionally collect until its reply succeeds.
        # At every other time this also closes the tiny handoff window between
        # a disconnecting dispatcher and its replacement.
        if self.opened:
            self.dispatching()

    def dispatching(self) -> None:
        with self._dispatch_lock:
            self._dispatching_locked()

    def _dispatching_locked(self) -> None:
        if self.caller and self.caller.is_alive():
            return
        caller = threading.Thread(
            target=self.deliver, daemon=True, name=f"omni:{self.session_id}"
        )
        self.caller = caller
        caller.start()

    def deliver(self) -> None:
        """Hand events to callbacks, one at a time, in the order they happened."""
        current = threading.current_thread()
        try:
            while True:
                arrived = self.inbox.get()
                if arrived is None:
                    return
                generation, frame = arrived
                # Synchronous open can receive replay before its reply. Do not
                # expose those callbacks until success, and discard them if the
                # open is refused.
                generation.ready.wait()
                with self._start_lock:
                    if not generation.accepted or generation is not self._generation:
                        continue
                    if frame.get("stream") == "disconnected":
                        # A dead daemon is not a stopped conversation. Its log
                        # remains on disk for this object's next generation.
                        self.opened = False
                        with self._state_lock:
                            self._awaiting.clear()
                            self.state["status"] = WAITING
                            self.state["in_turn"] = False
                        return
                    if frame.get("stream") == "gone":
                        with self._state_lock:
                            self._awaiting.clear()
                            snapshot = frame.get("snapshot")
                            self.state = (
                                _normalize_snapshot(snapshot) if snapshot else self.state
                            )
                            self.state["status"] = STOPPED
                        self.opened = False
                        self.finished = True
                        return
                    # The daemon says where the session stands as each event
                    # goes out, so nothing here has to infer it a second time.
                    event = Event.from_dict(frame.get("event") or {})
                    with self._state_lock:
                        snapshot = frame.get("snapshot")
                        self.state = _normalize_snapshot(snapshot) if snapshot else self.state
                        if event.seq > self.seen:
                            self.seen = event.seq
                        if event.type in (Event.START, Event.INJECTED):
                            for index, ticket in enumerate(self._awaiting):
                                if (
                                    event.seq > ticket["after"]
                                    and event.text == ticket["text"]
                                ):
                                    self._awaiting.pop(index)
                                    break
                        elif (
                            event.type == Event.ERROR
                            and self.state.get("status") == WAITING
                            and not self.state.get("in_turn")
                        ):
                            # The daemon accepted the request but could not hand
                            # any queued message to a provider. No opening event
                            # will arrive for those local busy tickets.
                            self._awaiting.clear()
                        stopped = self.state.get("status") == STOPPED
                        if stopped:
                            self._awaiting.clear()
                    if stopped:
                        self.opened = False
                        self.finished = True
                # Never hold the lifecycle lock across user code: reconnecting
                # must not wait for a slow callback, and callbacks may themselves
                # call start/send.
                self.events.emit(event)
                self.log.emit(event)
        finally:
            # Coordinate with `dispatching`: if start observed this thread just
            # before it exited, hand ownership directly to a replacement.
            with self._dispatch_lock:
                if self.caller is current:
                    self.caller = None
                    if self.opened and not self.finished:
                        self._dispatching_locked()

    def settle_down(self) -> None:
        # Lifecycle methods must not start a daemon merely in order to leave
        # it. In particular, ``stop`` and ``detach`` are harmless on a Chat
        # whose first ``start`` failed.
        if self.connection is not None:
            self.connection.unlisten(self.session_id)
        with self._state_lock:
            self._awaiting.clear()
        if self.caller and self.caller.is_alive():
            self.inbox.put(None)
            if self.caller is not threading.current_thread():
                self.caller.join(timeout=5)

    def __repr__(self) -> str:
        return f"<Chat {self.session_id} {self.status} provider={self.provider!r}>"
