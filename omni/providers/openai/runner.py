"""Running Codex for one omni session.

A thread is Codex's session. omni starts one, or resumes the one this omni
session already owns, and seeds anything it missed with ``thread/inject_items``
— the stable path, verified to make the model answer from history it never saw
live.

A message that arrives mid-turn is steered into the running turn rather than
queued behind it, which is the same promise omni makes everywhere else.
"""

import os

from ...events import Event
from ...translate import transcript
from .. import base
from . import jail
from .appserver import AppServer, AppServerError
from .stream import Stream

SANDBOX = "danger-full-access"
APPROVALS = "never"

#: Never a switch: ``AGENTS.md`` and friends would make the same run mean
#: different things depending on whose directory it started in.
NO_MEMORIES = ["-c", "project_doc_max_bytes=0"]
NO_SUBAGENTS = ["--disable", "apps", "--disable", "plugins", "-c", "agents.enabled=false"]


def items(turns: list[dict]) -> list[dict]:
    """omni turns as Codex ``ResponseItem``s."""
    out = []
    for turn in turns:
        user = turn["role"] == "user"
        out.append(
            {
                "type": "message",
                "role": turn["role"],
                "content": [
                    {"type": "input_text" if user else "output_text", "text": turn["text"]}
                ],
            }
        )
    return out


class Runner(base.Runner):
    name = "openai"
    cli = "codex app-server"

    def __init__(self, session_id, config, emit):
        super().__init__(session_id, config, emit)
        self.stream = Stream(config.model)
        self.server: AppServer | None = None
        self.turn_id = ""

    # ------------------------------------------------------------- lifecycle

    def start(self, native_id: str = "", history=None) -> None:
        self.announce()
        home = jail.build(self.session_id)
        self.server = AppServer(
            env=dict(os.environ, CODEX_HOME=str(home)),
            cwd=self.config.cwd,
            on_notify=self.notified,
            argv=self.flags(),
            on_exit=self.exited,
        )
        self.server.start()
        self.native_id = self.resume(native_id) if native_id else self.open()
        if self.config.disable_subagents:
            self.silence_skills()
        seed = transcript(history or [])
        if seed:
            self.server.call("thread/inject_items", {"threadId": self.native_id, "items": items(seed)})

    def announce(self) -> None:
        """MCP is not a switch here, so say so rather than let a caller believe it.

        CODEX_HOME is redirected whether or not it was asked for — the jail *is*
        how omni isolates codex. A chat that opts back into MCP still does not
        get it, and quietly not getting it is the worst of the options.
        """
        if self.config.disable_mcp:
            return
        self.emit(
            Event(
                type=Event.CONFIG,
                provider=self.name,
                text="unsupported",
                extra={
                    "ignored": ["enable_mcp"],
                    "why": "codex always runs in a jailed CODEX_HOME, so MCP cannot load",
                },
            )
        )

    def flags(self) -> list[str]:
        """MCP is already gone with the jail; this is everything else.

        Project docs go regardless. Subagents are the only part you can ask for
        back, and asking must not quietly bring the memories with them.
        """
        return (NO_SUBAGENTS if self.config.disable_subagents else []) + NO_MEMORIES

    def settings(self) -> dict:
        body = {
            "cwd": self.config.cwd,
            "sandbox": SANDBOX,
            "approvalPolicy": APPROVALS,
        }
        if self.config.model:
            body["model"] = self.config.model
        if self.config.system_prompt:
            body["baseInstructions"] = self.config.system_prompt
        if self.config.append_system_prompt:
            body["developerInstructions"] = self.config.append_system_prompt
        return body

    def open(self) -> str:
        started = self.server.call("thread/start", self.settings(), timeout=60)
        return (started.get("thread") or {}).get("id", "")

    def resume(self, native_id: str) -> str:
        """Pick the thread back up. If Codex has lost it, start a clean one."""
        try:
            back = self.server.call("thread/resume", dict(self.settings(), threadId=native_id), timeout=60)
        except AppServerError as exc:
            self.emit(
                Event(
                    type=Event.ERROR,
                    provider=self.name,
                    kind="crash",
                    ok=False,
                    error=f"could not resume codex thread {native_id}: {exc}",
                )
            )
            return self.open()
        return (back.get("thread") or {}).get("id", native_id)

    def silence_skills(self) -> None:
        """Skills live outside CODEX_HOME, so the empty-folder trick misses them."""
        listing = self.server.try_call("skills/list", {"cwds": [self.config.cwd]}, timeout=30) or {}
        for group in listing.get("data") or []:
            for skill in group.get("skills") or []:
                if skill.get("enabled"):
                    self.server.try_call(
                        "skills/config/write", {"name": skill.get("name"), "enabled": False}
                    )

    # ------------------------------------------------------------------- io

    def notified(self, method: str, params: dict) -> None:
        if method == "turn/started":
            self.turn_id = (params.get("turn") or {}).get("id", "")
        for event in self.stream.feed(method, params):
            self.emit(event)
        if method == "turn/completed":
            self.turn_id = ""

    def send(self, text: str) -> None:
        body = {"input": [{"type": "text", "text": text}]}
        if self.turn_id and self.steer(body):
            return
        body.update(threadId=self.native_id, summary="none")
        if self.config.model:
            body["model"] = self.config.model
        if self.config.effort:
            body["effort"] = self.config.effort
        self.server.call("turn/start", body, timeout=60)

    def steer(self, body: dict) -> bool:
        """Land a message inside the running turn.

        The turn can finish between reading its id and codex hearing about it,
        so a refusal means open a new turn instead of losing the message.
        """
        steered = dict(body, threadId=self.native_id, expectedTurnId=self.turn_id)
        return self.server.try_call("turn/steer", steered, timeout=30) is not None

    def retune(self, model: str, effort: str) -> bool:
        """Codex takes model and effort per turn, so this costs nothing at all."""
        self.config.model, self.config.effort = model, effort
        self.stream.model = model
        return self.alive

    def interrupt(self) -> None:
        if self.alive and self.turn_id:
            self.server.try_call(
                "turn/interrupt", {"threadId": self.native_id, "turnId": self.turn_id}, timeout=10
            )

    def stop(self) -> None:
        self.stopping = True
        if self.server:
            self.server.stop()

    @property
    def alive(self) -> bool:
        return bool(self.server and self.server.alive)
