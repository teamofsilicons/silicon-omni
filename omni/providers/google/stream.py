"""agy's stream-json, turned into omni events.

Everything arrives as ``step_update`` lines keyed by ``step_index``: a step goes
``ACTIVE`` then ``DONE``, and assistant text comes as deltas that have to be
stitched back together. Short answers sometimes skip ``ACTIVE`` entirely, so
nothing here assumes a step was announced before it finished.
"""

import json

from ...events import Event, classify


class Stream:
    def __init__(self, model: str = ""):
        self.model = model
        self.conversation = ""
        self.text: dict[int, str] = {}
        self.called: set[int] = set()

    def event(self, type_: str, **fields) -> Event:
        return Event(type=type_, provider="google", model=self.model, **fields)

    def feed(self, line: str) -> list[Event]:
        try:
            data = json.loads(line)
        except json.JSONDecodeError:
            return []
        name = data.get("event")
        if name == "init":
            self.conversation = data.get("conversation_id", self.conversation)
            return []
        if name == "step_update":
            return self.step(data.get("step_update") or {})
        if name == "result":
            return self.finished(data.get("result") or {})
        return []

    def step(self, step: dict) -> list[Event]:
        kind = step.get("step_type")
        if kind == "agent_response":
            return self.speech(step)
        if kind == "tool":
            return self.tool(step)
        return []

    def speech(self, step: dict) -> list[Event]:
        index = step.get("step_index", -1)
        self.text[index] = self.text.get(index, "") + (step.get("text_delta") or "")
        if step.get("state") == "ACTIVE":
            return []
        events = []
        if (step.get("usage") or {}).get("thinking_tokens"):
            events.append(self.event(Event.THINKING))
        said = self.text.pop(index, "").strip()
        if said:
            events.append(self.event(Event.TEXT, text=said))
        return events

    def tool(self, step: dict) -> list[Event]:
        index = step.get("step_index", -1)
        info = step.get("tool_info") or {}
        name = step.get("tool_name") or info.get("name") or "tool"
        events = []
        if index not in self.called:
            self.called.add(index)
            events.append(
                self.event(
                    Event.TOOL.CALL, tool=name, id=str(index), args=info.get("parameters") or {}
                )
            )
        if step.get("state") == "ACTIVE":
            return events
        self.called.discard(index)
        failure = info.get("error") or {}
        events.append(
            self.event(
                Event.TOOL.RESULT,
                tool=name,
                id=str(index),
                result=info.get("output") if not failure else failure.get("message"),
                ok=not failure and step.get("state") != "ERROR",
            )
        )
        return events

    def finished(self, result: dict) -> list[Event]:
        events = []
        if result.get("status") == "ERROR":
            events.append(
                self.event(
                    Event.ERROR,
                    kind=classify(result.get("error") or ""),
                    ok=False,
                    error=str(result.get("error") or "agy turn failed"),
                )
            )
        events.append(
            self.event(
                Event.END,
                extra={
                    "status": result.get("status"),
                    "seconds": result.get("duration_seconds"),
                    "usage": result.get("usage"),
                },
            )
        )
        return events



def user_line(text: str) -> str:
    """agy keys its input on ``event``, not ``type``."""
    return json.dumps({"event": "user", "message": {"role": "user", "content": text}})
