"""Claude's ``--output-format stream-json``, turned into omni events.

One line in, zero or more events out. The parser holds just enough state to
name a tool result after the call it belongs to, and to remember the session id
and model Claude reports at the top of every turn.

Thinking blocks arrive with a signature and are encrypted; omni notes that the
model thought and throws the content away.
"""

import json

from ...events import Event, classify

TEXT_BLOCKS = ("text",)
THINKING_BLOCKS = ("thinking", "redacted_thinking")


class Stream:
    def __init__(self, model: str = ""):
        self.tools: dict[str, str] = {}
        self.session_id = ""
        self.model = model
        self.rate_limit: dict | None = None

    def event(self, type_: str, **fields) -> Event:
        return Event(type=type_, provider="claude", model=self.model, **fields)

    def feed(self, line: str) -> list[Event]:
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            return []
        kind = data.get("type")
        if kind == "system":
            return self.system(data)
        if kind == "assistant":
            return self.assistant(data)
        if kind == "user":
            return self.results(data)
        if kind == "rate_limit_event":
            return self.rate_limited(data)
        if kind == "result":
            return self.finished(data)
        return []

    def system(self, data: dict) -> list[Event]:
        """``init`` opens every turn and carries the session id and model."""
        if data.get("subtype") == "init":
            self.session_id = data.get("session_id", self.session_id)
            self.model = data.get("model", self.model)
        return []

    def assistant(self, data: dict) -> list[Event]:
        message = data.get("message") or {}
        self.model = message.get("model", self.model)
        events = []
        for block in message.get("content") or []:
            kind = block.get("type")
            if kind in TEXT_BLOCKS and block.get("text"):
                events.append(self.event(Event.TEXT, text=block["text"]))
            elif kind in THINKING_BLOCKS:
                events.append(self.event(Event.THINKING))
            elif kind == "tool_use":
                self.tools[block.get("id", "")] = block.get("name", "")
                events.append(
                    self.event(
                        Event.TOOL.CALL,
                        tool=block.get("name", ""),
                        id=block.get("id", ""),
                        args=block.get("input") or {},
                    )
                )
        return events

    def results(self, data: dict) -> list[Event]:
        """A ``user`` line from Claude is a tool result coming back."""
        content = (data.get("message") or {}).get("content")
        if not isinstance(content, list):
            return []
        events = []
        for block in content:
            if block.get("type") != "tool_result":
                continue
            call_id = block.get("tool_use_id", "")
            events.append(
                self.event(
                    Event.TOOL.RESULT,
                    tool=self.tools.pop(call_id, ""),
                    id=call_id,
                    result=flatten_content(block.get("content")),
                    ok=not block.get("is_error"),
                )
            )
        return events

    def rate_limited(self, data: dict) -> list[Event]:
        self.rate_limit = data.get("rate_limit_info") or {}
        if self.rate_limit.get("status") == "rejected":
            return [
                self.event(
                    Event.ERROR,
                    kind="limit",
                    ok=False,
                    error=f"rate limited ({self.rate_limit.get('rateLimitType', '?')})",
                    extra=self.rate_limit,
                )
            ]
        return []

    def finished(self, data: dict) -> list[Event]:
        """``result`` closes the turn, successfully or not."""
        events = []
        if data.get("is_error") or data.get("subtype") != "success":
            events.append(
                self.event(
                    Event.ERROR,
                    kind=classify(
                        f"{data.get('subtype', '')} {data.get('result', '')} "
                        f"{data.get('api_error_status', '')}"
                    ),
                    ok=False,
                    error=str(data.get("result") or data.get("subtype") or "unknown error"),
                )
            )
        events.append(
            self.event(
                Event.END,
                extra={
                    "stop_reason": data.get("stop_reason"),
                    "cost_usd": data.get("total_cost_usd"),
                    "usage": data.get("usage"),
                    "turns": data.get("num_turns"),
                },
            )
        )
        return events


def flatten_content(content) -> str:
    """Tool output arrives as a string or a list of blocks; omni wants text."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(
            block.get("text", "") if isinstance(block, dict) else str(block) for block in content
        ).strip()
    return "" if content is None else str(content)
