"""Running Claude Code for one omni session.

One long-lived ``claude -p`` process handles every turn: messages go in as
NDJSON on stdin, events come back on stdout, and the process stays up between
turns. A message written while a turn is in flight is picked up by Claude at
the next safe point, which is exactly the injection behaviour omni promises.

Seeding is a file write before launch (see :mod:`.session`), so switching to
Claude costs nothing but the disk.
"""

import itertools
import json
import os
import threading
import uuid

from ...events import Event
from ...shared.proc import LineProcess
from ...translate import transcript
from .. import base
from . import control, session
from .stream import Stream

# Always on: memory files would make the same run mean different things on
# different machines. Subagents and MCP are opt-out per session, these are not.
QUIET_ENV = {
    "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
    "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
    "CLAUDE_CODE_DISABLE_ORG_MEMORY": "1",
}


class Runner(base.Runner):
    name = "claude"
    cli = "claude"

    def __init__(self, session_id, config, emit):
        super().__init__(session_id, config, emit)
        self.stream = Stream(config.model)
        self.proc: LineProcess | None = None
        self.waiting: dict[str, list] = {}
        self.tickets = itertools.count(1)

    def start(self, native_id: str = "", history=None) -> None:
        # Resume is scoped to the working directory: a session file that is not
        # under this cwd's slug does not exist as far as claude is concerned.
        # Reporting a different id back is how omni learns to reseed in full.
        lost = bool(native_id) and not session.session_path(self.config.cwd, native_id).exists()
        self.native_id = ("" if lost else native_id) or str(uuid.uuid4())
        turns = [] if lost else transcript(history or [])
        if turns:
            session.seed(self.config.cwd, self.native_id, turns, self.config.model)
        known = session.session_path(self.config.cwd, self.native_id).exists()
        self.proc = LineProcess(
            self.argv(resume=known),
            on_line=self.line,
            on_stderr=self.problem,
            on_exit=self.exited,
            env=self.environment(),
            cwd=self.config.cwd,
        ).start()

    def argv(self, resume: bool) -> list[str]:
        argv = [
            "claude",
            "-p",
            "--output-format", "stream-json",
            "--input-format", "stream-json",
            "--verbose",
            "--dangerously-skip-permissions",
            "--disable-slash-commands",
        ]
        argv += ["--resume", self.native_id] if resume else ["--session-id", self.native_id]
        if self.config.model:
            argv += ["--model", self.config.model]
        if self.config.effort:
            argv += ["--effort", self.config.effort]
        if self.config.system_prompt:
            argv += ["--system-prompt", self.config.system_prompt]
        if self.config.append_system_prompt:
            argv += ["--append-system-prompt", self.config.append_system_prompt]
        if self.config.disable_subagents:
            argv += ["--disallowedTools", "Agent(*)"]
        if self.config.disable_mcp:
            argv += ["--strict-mcp-config", "--setting-sources", ""]
        return argv

    def environment(self) -> dict:
        env = dict(os.environ, **QUIET_ENV)
        if self.config.disable_subagents:
            env["CLAUDE_CODE_DISABLE_WORKFLOWS"] = "1"
        return env

    # ------------------------------------------------------------------- io

    def line(self, text: str) -> None:
        if self.waiting and self.answered(text):
            return
        for event in self.stream.feed(text):
            self.emit(event)

    def answered(self, text: str) -> bool:
        """Hand a control response back to whoever is waiting on it."""
        if '"control_response"' not in text:
            return False
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            return False
        if data.get("type") != "control_response":
            return False
        response = data.get("response") or {}
        slot = self.waiting.pop(response.get("request_id", ""), None)
        if not slot:
            return False
        slot[0] = response
        slot[1].set()
        return True

    def ask(self, subtype: str, timeout: float = 15.0, **fields) -> bool:
        """A control request omni only believes once the CLI says it worked."""
        request_id = f"omni-{subtype}-{next(self.tickets)}"
        slot = [None, threading.Event()]
        self.waiting[request_id] = slot
        if not self.proc.send_line(control.line(subtype, request_id, **fields)):
            self.waiting.pop(request_id, None)
            return False
        if not slot[1].wait(timeout):
            self.waiting.pop(request_id, None)
            return False
        return slot[0].get("subtype") == "success"

    def problem(self, text: str) -> None:
        """Claude warns on stderr. Worth surfacing, never worth a teardown.

        Only :meth:`exited` reports a crash — a line that merely says "error"
        may be a deprecation notice or a tool's own output.
        """
        if text.lower().startswith("error"):
            self.emit(Event(type=Event.ERROR, provider=self.name, kind="stderr", ok=False, error=text))

    def send(self, text: str) -> None:
        if self.proc:
            self.proc.send_line(session.user_line(text))

    def retune(self, model: str, effort: str) -> bool:
        """Swap model or effort over the control channel — no restart, no re-read.

        A rejected request means omni restarts instead, rather than reporting a
        model the CLI is not actually using.
        """
        if not self.alive:
            return False
        if model and model != self.config.model and not self.ask("set_model", model=model):
            return False
        if effort != self.config.effort:
            # An empty effort means "back to the default", and the flag layer is
            # replaced wholesale — so it has to be sent, not skipped.
            settings = {"effortLevel": effort} if effort else {}
            if not self.ask("apply_flag_settings", settings=settings):
                return False
        self.config.model, self.config.effort = model, effort
        self.stream.model = model
        return True

    def interrupt(self) -> None:
        if self.alive:
            self.proc.send_line(control.line("interrupt", "omni-interrupt"))

    def stop(self) -> None:
        self.stopping = True
        if self.proc:
            self.proc.stop(timeout=10)

    @property
    def alive(self) -> bool:
        return bool(self.proc and self.proc.alive)
