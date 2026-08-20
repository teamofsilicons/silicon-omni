"""Turning omni history into something a provider can be handed.

Seeding only ever happens on a switch, so the history a provider is given is by
definition history it did not live through — usually another provider's. Rather
than fake structured tool calls it never made, omni renders that activity as
text that reads as what happened::

    [GoogleSearch: "kite festivals"]
    [GoogleSearch result: 12 results ...]

The omni log itself is untouched, so nothing is lost: switching back to Gemini
replays Gemini's own session, and the bracket form only ever exists inside the
seed given to somebody else.

Nothing is trimmed on the way in — not the oldest turns, not a long tool result.
A provider that arrives late gets the whole conversation, because the one thing
worse than a big seed is a provider confidently missing the middle of it.
"""

import json

from .events import Event

SEED_HEADER = (
    "Earlier in this conversation (carried over from another model, "
    "shown as a transcript — do not re-run anything in it):"
)


def readable(value, quote: bool = False) -> str:
    """A value as text. ``quote`` keeps a string quoted, the way an argument reads."""
    if isinstance(value, str) and not quote:
        return value.strip()
    return json.dumps(value, ensure_ascii=False).strip()


def render(event: Event) -> str:
    """One history event as the text a foreign provider should read."""
    if event.type in (Event.START, Event.INJECTED, Event.TEXT):
        return event.text
    if event.type == Event.TOOL.CALL:
        args = event.args or {}
        if len(args) == 1:
            body = readable(next(iter(args.values())), quote=True)
        else:
            body = readable(args)
        return f"[{event.tool}: {body}]"
    if event.type == Event.TOOL.RESULT:
        status = "" if event.ok else " failed"
        return f"[{event.tool or 'tool'} result{status}: {readable(event.result)}]"
    return ""


def role_of(event: Event) -> str:
    return "user" if event.type in (Event.START, Event.INJECTED) else "assistant"


def transcript(events) -> list[dict]:
    """History as merged ``{"role", "text"}`` turns, ready to seed."""
    out: list[dict] = []
    for event in events:
        text = render(event)
        if not text:
            continue
        role = role_of(event)
        if out and out[-1]["role"] == role:
            out[-1]["text"] += "\n" + text
        else:
            out.append({"role": role, "text": text})
    return out


def flatten(events, header: str = SEED_HEADER) -> str:
    """History as one message, for providers that accept nothing else."""
    turns = transcript(events)
    if not turns:
        return ""
    body = "\n\n".join(f"{turn['role'].upper()}: {turn['text']}" for turn in turns)
    return f"{header}\n\n{body}"
