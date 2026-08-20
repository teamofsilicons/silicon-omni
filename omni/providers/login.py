"""Driving a CLI's own login, so nobody has to open the CLI.

All three do the same dance: print a URL, wait for you to open it, then either
finish over a local callback or ask you to paste a code back. omni spawns the
login, hands you the URL, and types the code for you when there is one.

If the CLI does something else entirely, whatever it printed is handed back
verbatim — a confusing message you can read beats a silent failure.
"""

import re
import threading
import time

from ..shared.proc import LineProcess

URL = re.compile(r"https?://[^\s\"'<>()\]]+")


class Login:
    """One in-flight login attempt."""

    def __init__(self, argv: list[str], env: dict | None = None):
        self.argv = argv
        self.env = env
        self.output: list[str] = []
        self.seen = threading.Event()
        self.url = ""
        self.proc: LineProcess | None = None

    def capture(self, line: str) -> None:
        self.output.append(line)
        found = URL.search(line)
        if found and not self.url:
            self.url = found.group(0)
            self.seen.set()

    def start(self, timeout: float = 45.0) -> str:
        """Begin the login. Returns the URL to open, or whatever went wrong."""
        self.proc = LineProcess(self.argv, on_line=self.capture, on_stderr=self.capture, env=self.env)
        self.proc.start()
        self.seen.wait(timeout)
        if self.url:
            return self.url
        self.stop()
        manual = f"omni could not drive this login. Run it yourself: {' '.join(self.argv)}"
        return f"{self.transcript}\n\n{manual}".strip()

    def finish(self, code: str, timeout: float = 180.0) -> None:
        """Hand the code (or redirect URL) back to the CLI and wait for it to settle."""
        if self.proc and self.proc.alive and code:
            self.proc.send_line(code.strip())
        deadline = time.time() + timeout
        while time.time() < deadline and self.proc and self.proc.alive:
            time.sleep(0.25)
        self.stop()

    def stop(self) -> None:
        if self.proc:
            self.proc.stop(timeout=3)

    @property
    def transcript(self) -> str:
        return "\n".join(self.output)
