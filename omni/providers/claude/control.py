"""Claude's stdin control channel.

``--input-format stream-json`` accepts control requests alongside user
messages. They are how omni changes model or effort without tearing the process
down and re-reading the whole conversation, and how it reads usage without
spending a token.
"""

import json
import threading

from ...shared.proc import LineProcess

BARE = ["claude", "-p", "--output-format", "stream-json", "--input-format", "stream-json", "--verbose"]


def line(subtype: str, request_id: str = "omni", **fields) -> str:
    return json.dumps(
        {"type": "control_request", "request_id": request_id, "request": {"subtype": subtype, **fields}}
    )


def ask(subtype: str, timeout: float = 30.0, **fields) -> dict | None:
    """Ask a throwaway ``claude`` process one question. Costs no tokens."""
    answer: dict = {}
    done = threading.Event()

    def read(text: str) -> None:
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            return
        if data.get("type") == "control_response":
            answer.update(data.get("response") or {})
            done.set()

    proc = LineProcess(BARE, on_line=read).start()
    proc.send_line(line(subtype, **fields))
    done.wait(timeout)
    proc.stop(timeout=5)
    return answer.get("response") if answer.get("subtype") == "success" else None
