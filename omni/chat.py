"""One omni session, whichever provider happens to be running it.

Everything that matters happens on a single conductor thread: provider output,
user messages and lifecycle changes all arrive on one queue and are handled in
order. That is why callbacks fire one at a time, in the order things actually
happened, and why nothing ever changes mid-turn.

The rule the whole design hangs off: **nothing changes mid turn**. Intelligence,
providers, prompts and session swaps are recorded when you ask for them and
applied at the next turn boundary.
"""

import os
import queue
import threading
from dataclasses import asdict, replace
from typing import Callable, Sequence

from .events import AUTH, CRASH, Event
from .intelligence import resolve
from .providers import account, runner_for
from .providers.base import Config
from .session import Lock, Meta, Store
from .shared.bus import Bus

IDLE = "idle"
WAITING = "waiting"
BUSY = "busy"
STOPPED = "stopped"

MODEL_EVENTS = (Event.TEXT, Event.THINKING, Event.TOOL.CALL, Event.TOOL.RESULT)

#: Errors that end the turn rather than just being reported.
FATAL = (AUTH, CRASH)

#: Notices that are true of a provider rather than of a moment. Worth saying
#: when you ask for the thing, and when the conversation arrives somewhere that
#: cannot do it — not every time a process restarts underneath it.
ONCE = ("unsupported", "approximated")


class Chat:
    """A persistent conversation. Get one from ``Inference.load_or_create_session``."""

    def __init__(self, session_id: str, providers: Sequence[str]):
        self.session_id = session_id
        self.store = Store(session_id)
        self.meta = Meta(session_id)
        self.lock = Lock(session_id).acquire()
        self.providers = list(providers)
        self.level = int(self.meta.get("level", 5))
        self.config = Config()
        # Claude resumes by cwd, so a session that moves directory loses its
        # provider sessions. Pin the directory to the session the first time.
        self.config.cwd = self.meta.get("cwd") or self.config.cwd
        self.meta.set("cwd", self.config.cwd)
        self.events = Bus(on_error=self.handler_failed)
        self.log = Bus(on_error=self.log_failed)
        self.inbox: queue.Queue = queue.Queue()
        self.conductor: threading.Thread | None = None
        self.runner = None
        self.current = None  # the (provider, model, effort, config) the runner was built with
        self.outbox: list[str] = []
        self.in_turn = False
        self.stopping = False
        self.autoremove = True
        self.announced: set[tuple] = set()
        self.state = IDLE

    # ------------------------------------------------------------------ setup

    def active_inference_providers(self, providers: Sequence[str]) -> None:
        """Limit which providers this chat may use. Applied at the next turn boundary."""
        self.providers = list(providers)
        self.note("providers", providers=self.providers)

    def intelligence(self, level: int) -> None:
        """0-10 across every active provider. May change model *and* provider."""
        self.level = int(level)
        self.meta.set("level", self.level)
        self.note("intelligence", level=self.level)

    #: the spelling used in the README's example
    inteligence = intelligence

    def system_prompt(self, text: str) -> None:
        """Replace the provider's own session prompt."""
        self.config.system_prompt = text
        self.note("system_prompt", chars=len(text))

    def system_prompt_file(self, path: str) -> None:
        self.system_prompt(open(path, encoding="utf-8").read())

    def append_system_prompt(self, text: str) -> None:
        """Keep the provider's prompt and add to it."""
        self.config.append_system_prompt = text
        self.note("append_system_prompt", chars=len(text))

    def append_system_prompt_file(self, path: str) -> None:
        self.append_system_prompt(open(path, encoding="utf-8").read())

    def disable_subagents(self) -> None:
        """No provider-side subagents, so only the workers you define get used.

        Already the default; here so asking for it out loud still reads.
        """
        self.isolation("subagents", False)

    def enable_subagents(self) -> None:
        """Let the provider spawn its own subagents. Off unless you ask."""
        self.isolation("subagents", True)

    def disable_mcp(self) -> None:
        """No MCP servers, no external connectors. Already the default."""
        self.isolation("mcp", False)

    def enable_mcp(self) -> None:
        """Let the provider load its MCP servers and connectors. Off unless you ask.

        Not every provider can honour it — codex is always jailed and agy has no
        switch at all — and the one that cannot says so.
        """
        self.isolation("mcp", True)

    def isolation(self, what: str, on: bool) -> None:
        setattr(self.config, f"disable_{what}", not on)
        self.announced.clear()  # whether a provider can honour this may have changed
        self.note(what, on=on)

    def disable_autoremoving_unauthenticated_providers(self) -> None:
        """Stop dropping a provider that loses its login mid-run.

        On by default: an unauthenticated CLI cannot finish the turn, so omni
        takes it off this chat's list and resolves the same intelligence level
        again over whoever is left. Turn it off and the auth error is reported
        and the turn simply ends.
        """
        self.autoremove = False
        self.note("autoremove_unauthenticated", on=False)

    def cwd(self, path: str) -> None:
        """Where the provider runs its tools.

        Pinned to the session: Claude resumes by working directory, so moving a
        session costs it Claude's own history and it gets seeded again.
        """
        self.config.cwd = os.path.realpath(path)
        self.meta.set("cwd", self.config.cwd)
        self.note("cwd", cwd=self.config.cwd)

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
        """A broken log handler cannot be reported to the log handlers.

        It goes straight to the session file instead, so the failure is on disk
        rather than nowhere.
        """
        self.store.append(self.blame(exc, fn))

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
        return self.state

    @property
    def idle(self) -> bool:
        """Waiting, with nothing left to process. What a polling loop should check."""
        return self.state == WAITING and self.inbox.empty()

    def start(self) -> "Chat":
        if self.stopping or self.state == STOPPED:
            raise RuntimeError(f"session {self.session_id!r} is stopped; load it again to continue")
        if self.conductor and self.conductor.is_alive():
            return self
        self.state = WAITING
        self.conductor = threading.Thread(target=self.run, daemon=True, name=f"omni:{self.session_id}")
        self.conductor.start()
        self.inbox.put(("launch", None))
        return self

    def send(self, text: str) -> None:
        """Send a message. Opens a turn, or lands inside the one already running.

        Returns immediately; ``status`` flips to ``busy`` before it does, so a
        caller polling in a loop never sees a false lull.
        """
        if self.stopping or self.state == STOPPED:
            raise RuntimeError(f"session {self.session_id!r} is stopped; load it again to continue")
        if self.state in (WAITING, BUSY):
            self.state = BUSY
        self.inbox.put(("send", text))

    def stop(self) -> None:
        """End the chat: the provider is shut down and the session id is released.

        Safe to call from inside an event handler — it will not wait on itself.
        """
        if self.stopping:
            return
        self.stopping = True
        running = self.conductor and self.conductor.is_alive()
        if not running:
            self.lock.release()  # never started, or already gone: let the id go
        else:
            self.inbox.put(("stop", None))
            if self.conductor is not threading.current_thread():
                self.conductor.join(timeout=30)
        self.state = STOPPED

    def __enter__(self) -> "Chat":
        return self.start()

    def __exit__(self, *exc) -> None:
        self.stop()

    # -------------------------------------------------------------- conductor

    def run(self) -> None:
        """The one thread that owns this chat's state."""
        while True:
            kind, payload = self.inbox.get()
            try:
                if kind == "stop":
                    self.shutdown()
                    return
                if kind == "launch":
                    self.rebuild()
                elif kind == "send":
                    self.dispatch(payload)
                elif kind == "event":
                    self.absorb(payload)
            except Exception as exc:  # a bad turn must not kill the chat
                self.record(Event(type=Event.ERROR, kind="omni", ok=False, error=repr(exc)))

    def push(self, event: Event) -> None:
        """Runners call this from their own threads; the conductor does the work."""
        self.inbox.put(("event", event))

    def absorb(self, event: Event) -> None:
        if self.repeated(event):
            return
        self.record(event)
        if self.stopping or self.straggler(event):
            return  # still logged, but there is nothing left to react to
        if event.type in MODEL_EVENTS:
            self.state = BUSY
        elif event.type == Event.END:
            self.in_turn = False
            self.finish_turn()
        elif event.type == Event.ERROR and event.kind in FATAL:
            self.fatal(event)

    def repeated(self, event: Event) -> bool:
        """Has this provider already told us it cannot do this?

        Forgotten when the setting changes and when the conversation moves to
        another provider, which are the two moments the answer can differ.
        """
        if event.type != Event.CONFIG or event.text not in ONCE:
            return False
        # repr, not a tuple: extra holds lists, and a tuple around a list
        # cannot be hashed — which would make this raise instead of dedupe.
        said = (event.text, event.provider, repr(sorted(event.extra.items(), key=str)))
        if said in self.announced:
            return True
        self.announced.add(said)
        return False

    def straggler(self, event: Event) -> bool:
        """Did this come from a provider omni has already moved on from?

        A dying CLI reports the failure and the end of its turn in one breath.
        By the time the second one is handled there may be a new provider up,
        and applying it there would close a turn that is still open — or take
        down the runner that has just taken over.
        """
        return bool(self.runner and event.provider and event.provider != self.runner.name)

    def fatal(self, event: Event) -> None:
        """An error that ends the turn: a crash, or a login that has gone."""
        if event.kind == AUTH:
            self.unauthenticated(event)
        else:
            self.collapse(event)

    def collapse(self, event: Event) -> None:
        """The provider died. Close the turn honestly and let the next send retry."""
        self.close_turn(event.provider, {"crashed": True})

    def unauthenticated(self, event: Event) -> None:
        """A provider lost its login mid-run. Take it off the dial and move on.

        The turn is over either way — an unauthenticated CLI cannot finish it.
        What differs is the next one: by default that provider is dropped from
        this chat and the same intelligence level is resolved again over the
        providers that are left, so the conversation carries on somewhere else.
        """
        name = event.provider or (self.runner.name if self.runner else "")
        self.close_turn(name, {"unauthenticated": name})
        if not (self.autoremove and name in self.providers):
            return
        self.providers.remove(name)
        account(name).forget()  # a login can come back; ask the CLI again next time
        self.record(
            Event(
                type=Event.CONFIG,
                text="provider_removed",
                provider=name,
                extra={"why": "unauthenticated", "left": list(self.providers)},
            )
        )
        if not self.providers:
            self.blocked("every provider is unauthenticated; log one back in and send again")
            return
        self.attempt("provider lost its login")
        self.flush()

    def close_turn(self, provider: str, extra: dict) -> None:
        """Put the runner down and end the turn it was in the middle of."""
        if self.runner:
            self.meta.mark_synced(self.runner.name, self.store.seq)
            self.runner.stop()
            self.runner = None
        if self.in_turn:
            self.in_turn = False
            self.record(Event(type=Event.END, provider=provider, extra=extra))
        self.state = WAITING

    def finish_turn(self) -> None:
        self.state = WAITING
        if self.runner:
            self.meta.mark_synced(self.runner.name, self.store.seq)
        if self.stale():
            self.attempt("settings changed")
        self.flush()

    def dispatch(self, text: str) -> None:
        """Queue a message, make sure something can carry it, then hand it over."""
        self.outbox.append(text)
        self.state = BUSY
        if self.runner is None or not self.runner.alive:
            self.attempt("no provider is running")
        elif not self.in_turn and self.stale():
            self.attempt("settings changed")  # between turns, so it can land now
        self.flush()

    def flush(self) -> None:
        """Hand queued messages over — never to a runner we are about to replace.

        A message is recorded only once the runner has taken it, so it is either
        seeded into a provider or sent to one, and never both, and a send that
        fails leaves the message queued rather than half-delivered.
        """
        if not (self.runner and self.runner.alive) or self.stale():
            self.stranded()
            return
        while self.outbox:
            text = self.outbox[0]
            try:
                self.runner.send(text)
            except Exception as exc:
                self.blocked(f"{self.runner.name} would not take the message: {exc!r}")
                return
            self.record(
                Event(type=Event.START if not self.in_turn else Event.INJECTED, text=text)
            )
            self.outbox.pop(0)
            self.in_turn = True
            self.state = BUSY

    def attempt(self, why: str) -> None:
        """Bring a provider up, and survive it refusing to come up."""
        try:
            self.rebuild()
        except Exception as exc:
            self.blocked(f"could not start a provider ({why}): {exc!r}")

    def stranded(self) -> None:
        if self.outbox and not (self.runner and self.runner.alive):
            self.blocked(f"nothing is running; {len(self.outbox)} message(s) still queued")

    def blocked(self, why: str) -> None:
        """Say what went wrong and go back to waiting, rather than hanging in 'busy'.

        Queued messages stay queued: the next send retries the whole thing.
        """
        self.record(Event(type=Event.ERROR, kind=CRASH, ok=False, error=why))
        self.state = WAITING

    # ---------------------------------------------------------------- runners

    def rung(self) -> dict:
        return resolve(self.level, self.providers)

    def signature(self, rung: dict) -> tuple:
        return (rung["provider"], rung["model"], rung["effort"], tuple(sorted(asdict(self.config).items())))

    def stale(self) -> bool:
        """Has anything been asked for that the running CLI cannot honour?"""
        return self.current is not None and self.current != self.signature(self.rung())

    def tunable(self, rung: dict) -> bool:
        """Is this only a model or effort change on the provider already running?"""
        if not (self.current and self.runner and self.runner.alive):
            return False
        provider, model, effort, config = self.current
        wanted = self.signature(rung)
        return provider == wanted[0] and config == wanted[3] and (model, effort) != wanted[1:3]

    def rebuild(self) -> None:
        """Bring up the provider the current settings ask for. Turn boundaries only."""
        rung = self.rung()
        if self.tunable(rung) and self.runner.retune(rung["model"], rung["effort"]):
            self.current = self.signature(rung)
            self.record(
                Event(
                    type=Event.CONFIG,
                    text="retune",
                    provider=rung["provider"],
                    model=rung["model"],
                    extra={"effort": rung["effort"], "level": rung["level"]},
                )
            )
            return
        # ``current`` outlives the runner, so a provider that crashed or lost
        # its login is still named as the one we came from.
        previous = self.runner.name if self.runner else (self.current[0] if self.current else "")
        if self.runner:
            # Its ``synced`` mark stays where the last turn left it: anything
            # recorded since then is exactly what it has to be told on the way back.
            self.meta.bind(previous, self.runner.native_id)
            self.runner.stop()
            self.runner = None
        if previous != rung["provider"]:
            self.announced.clear()  # a new provider gets to say what it cannot do
        if previous and previous != rung["provider"]:
            self.record(
                Event(
                    type=Event.SWITCH_PROVIDER,
                    provider=rung["provider"],
                    model=rung["model"],
                    extra={"from": previous, "to": rung["provider"], "level": rung["level"]},
                )
            )
        self.launch(rung)

    def launch(self, rung: dict) -> None:
        native = self.meta.native(rung["provider"])
        fresh = not native["id"]
        config = replace(self.config, model=rung["model"] or "", effort=rung["effort"] or "")
        runner = self.bring_up(rung, config, native["id"], native["synced"] + 1)
        if native["id"] and runner.native_id != native["id"]:
            # The provider no longer knows that session. Rather than carry on
            # with half a conversation, start clean and tell it everything.
            runner.stop()
            self.record(
                Event(
                    type=Event.CONFIG,
                    text="reseed",
                    provider=rung["provider"],
                    extra={"lost": native["id"], "now": runner.native_id},
                )
            )
            runner = self.bring_up(rung, config, "", 0)
            fresh = True
        self.runner = runner
        self.current = self.signature(rung)
        self.meta.bind(rung["provider"], runner.native_id)
        if runner.seeded:
            self.meta.mark_synced(rung["provider"], self.store.seq)
        self.record(
            Event(
                type=Event.CONFIG,
                text="launch",
                provider=rung["provider"],
                model=rung["model"],
                extra={"effort": rung["effort"], "level": rung["level"], "native": runner.native_id},
            )
        )
        if fresh:
            self.record(
                Event(
                    type=Event.NEW_SESSION,
                    provider=rung["provider"],
                    model=rung["model"],
                    extra={"native": runner.native_id},
                )
            )

    def bring_up(self, rung: dict, config, native_id: str, since: int):
        runner = runner_for(rung["provider"])(self.session_id, config, self.push)
        runner.start(native_id=native_id, history=self.store.history(since=since))
        return runner

    def shutdown(self) -> None:
        self.stopping = True
        if self.runner:
            if self.in_turn:
                self.runner.interrupt()  # ask nicely before closing the pipe
            self.meta.bind(self.runner.name, self.runner.native_id)
            self.runner.stop()
            self.runner = None
        self.drain()
        self.record(Event(type=Event.CONFIG, text="stop"))
        self.state = STOPPED
        self.lock.release()

    def drain(self) -> None:
        """Whatever the provider said on its way out still belongs in the log."""
        while True:
            try:
                kind, payload = self.inbox.get_nowait()
            except queue.Empty:
                return
            if kind == "event":
                self.record(payload)

    # ----------------------------------------------------------------- record

    def record(self, event: Event) -> Event:
        """Persist, then tell everyone. The session file is the event log."""
        event.session = event.session or self.session_id
        if self.runner and not event.provider:
            event.provider = self.runner.name
            event.model = event.model or self.runner.config.model
        self.store.append(event)
        self.events.emit(event)
        self.log.emit(event)
        return event

    def note(self, what: str, **extra) -> None:
        """A settings change. Persisted and logged; applied at the next boundary."""
        self.push(Event(type=Event.CONFIG, text=what, extra=extra))

    def __repr__(self) -> str:
        return f"<Chat {self.session_id} {self.state} level={self.level}>"
