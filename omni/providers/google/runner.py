"""Running Antigravity for one omni session.

agy cannot be seeded, so history it missed is flattened into text and folded
into the front of the next thing the user says — one turn, not two. Its own
conversations are resumed with ``--conversation``, but an id it no longer
recognises is silently replaced with a fresh one, so the id it reports back is
always checked against the one that was asked for.

Startup is slow (the CLI boots a language server every time), which is why
``start`` waits for the ``init`` line before handing back.
"""

import threading

from ...events import Event
from ...shared.proc import LineProcess
from ...translate import flatten
from .. import base
from .stream import Stream, user_line

READY = 120.0  # cold start is ~10s, but a loaded machine can take much longer
INSTRUCTIONS = "Follow these instructions for the rest of this conversation:"


class Runner(base.Runner):
    name = "google"
    cli = "agy"

    def __init__(self, session_id, config, emit):
        super().__init__(session_id, config, emit)
        self.stream = Stream(config.model)
        self.proc: LineProcess | None = None
        self.ready = threading.Event()
        self.seed = ""

    def start(self, native_id: str = "", history=None) -> None:
        self.announce()
        self.seed = self.opening(history or [])
        self.proc = LineProcess(
            self.argv(native_id),
            on_line=self.line,
            on_stderr=self.problem,
            on_exit=self.exited,
            cwd=self.config.cwd,
        ).start()
        self.ready.wait(READY)
        self.native_id = self.stream.conversation

    def announce(self) -> None:
        """Say which of omni's switches agy simply does not have.

        There is no flag for either, and its login is tied to the real home so
        the fake-home trick that works for codex is out. Better to say so than
        to let a caller believe subagents are off when they are not.
        """
        ignored = [
            name
            for name, on in (
                ("disable_subagents", self.config.disable_subagents),
                ("disable_mcp", self.config.disable_mcp),
            )
            if on
        ]
        if ignored:
            self.emit(
                Event(
                    type=Event.CONFIG,
                    provider=self.name,
                    text="unsupported",
                    extra={"ignored": ignored, "why": "agy has no switch for these"},
                )
            )

    def opening(self, history) -> str:
        """What agy has to be told before it can carry on: prompt, then history.

        There is no system prompt flag either, so the prompt goes in as text —
        an approximation, and omni says so rather than dropping it.
        """
        prompt = self.config.system_prompt or self.config.append_system_prompt
        if prompt:
            self.emit(
                Event(
                    type=Event.CONFIG,
                    provider=self.name,
                    text="approximated",
                    extra={"system_prompt": "sent as text; agy has no system prompt flag"},
                )
            )
        parts = [f"{INSTRUCTIONS}\n\n{prompt}" if prompt else "", flatten(history)]
        return "\n\n".join(part for part in parts if part)

    def argv(self, native_id: str) -> list[str]:
        argv = [
            "agy",
            "--output-format", "stream-json",
            "--input-format", "stream-json",
            "--disable-slash-commands",
            "--print-timeout", "24h",
            "--dangerously-skip-permissions",
        ]
        if self.config.model:
            argv += ["--model", self.config.model]
        if self.config.effort:
            argv += ["--effort", self.config.effort]
        if native_id:
            argv += ["--conversation", native_id]
        # --print takes a value, so it goes last with an explicit empty one.
        # Anywhere else it silently swallows the flag that follows it.
        return argv + ["--print", ""]

    def line(self, text: str) -> None:
        for event in self.stream.feed(text):
            self.emit(event)
        if self.stream.conversation and not self.ready.is_set():
            self.ready.set()

    def problem(self, text: str) -> None:
        """Only :meth:`exited` reports a crash; stderr is just agy talking."""
        if text.lower().startswith("error"):
            self.emit(Event(type=Event.ERROR, provider=self.name, kind="stderr", ok=False, error=text))

    @property
    def seeded(self) -> bool:
        """agy is only told anything when the next message goes out."""
        return not self.seed

    def exited(self, code: int) -> None:
        self.ready.set()  # nobody should wait out the cold-start timeout on a corpse
        super().exited(code)

    def send(self, text: str) -> None:
        if not self.proc:
            return
        if self.seed:
            text = f"{self.seed}\n\n---\n\n{text}"
            self.seed = ""
        self.proc.send_line(user_line(text))

    def stop(self) -> None:
        self.stopping = True
        if self.proc:
            self.proc.stop(timeout=10)

    @property
    def alive(self) -> bool:
        return bool(self.proc and self.proc.alive)
