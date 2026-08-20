"""Codex app-server notifications, turned into omni events.

Items are a tagged union that grows with every release, so only the four kinds
omni has an opinion about are named here — an assistant message, reasoning, the
user's own echo, and a shell command. Everything else, present and future, is
reported as a tool call by its own type name. New Codex tools work on the day
they ship, without a change here.
"""

from ...events import Event, classify

SKIP = ("userMessage", "hookPrompt")
SHELL = "commandExecution"
NOISE = ("id", "type", "status", "aggregatedOutput", "exitCode", "durationMs", "processId")


class Stream:
    def __init__(self, model: str = ""):
        self.model = model
        self.tokens: dict | None = None

    def event(self, type_: str, **fields) -> Event:
        return Event(type=type_, provider="openai", model=self.model, **fields)

    def feed(self, method: str, params: dict) -> list[Event]:
        if method == "item/started":
            return self.started(params.get("item") or {})
        if method == "item/completed":
            return self.completed(params.get("item") or {})
        if method == "turn/completed":
            return self.finished(params.get("turn") or {})
        if method == "error":
            return self.failed(params)
        if method == "thread/tokenUsage/updated":
            self.tokens = params
            return []
        return []

    def started(self, item: dict) -> list[Event]:
        kind = item.get("type")
        if kind in SKIP or kind == "agentMessage":
            return []
        if kind == "reasoning":
            return [self.event(Event.THINKING)]
        return [
            self.event(
                Event.TOOL.CALL,
                tool=tool_name(item),
                id=item.get("id", ""),
                args=arguments(item),
            )
        ]

    def completed(self, item: dict) -> list[Event]:
        kind = item.get("type")
        if kind in SKIP or kind == "reasoning":
            return []
        if kind == "agentMessage":
            text = item.get("text") or ""
            if not text:
                return []
            return [self.event(Event.TEXT, text=text, extra={"phase": item.get("phase")})]
        return [
            self.event(
                Event.TOOL.RESULT,
                tool=tool_name(item),
                id=item.get("id", ""),
                result=outcome(item),
                ok=item.get("exitCode") in (0, None) and item.get("status") != "failed",
            )
        ]

    def finished(self, turn: dict) -> list[Event]:
        events = []
        if turn.get("status") == "failed":
            error = turn.get("error") or {}
            events.append(
                self.event(
                    Event.ERROR,
                    kind=classify(f"{error.get('message', '')} {error.get('codexErrorInfo', '')}"),
                    ok=False,
                    error=str(error.get("message") or "turn failed"),
                    extra=error,
                )
            )
        events.append(
            self.event(
                Event.END,
                extra={"status": turn.get("status"), "ms": turn.get("durationMs"), "usage": self.tokens},
            )
        )
        return events

    def failed(self, params: dict) -> list[Event]:
        """A mid-turn error. Retryable ones do not end the turn."""
        return [
            self.event(
                Event.ERROR,
                kind=classify(f"{params.get('message', '')} {params.get('codexErrorInfo', '')}"),
                ok=False,
                error=str(params.get("message") or "codex error"),
                extra={"willRetry": params.get("willRetry")},
            )
        ]


def tool_name(item: dict) -> str:
    if item.get("type") == SHELL:
        return "shell"
    return item.get("type") or "tool"


def arguments(item: dict) -> dict:
    if item.get("type") == SHELL:
        return {"command": item.get("command", "")}
    return {key: value for key, value in item.items() if key not in NOISE}


def outcome(item: dict):
    if item.get("type") == SHELL:
        return item.get("aggregatedOutput")
    return {key: value for key, value in item.items() if key not in ("id", "type")}

