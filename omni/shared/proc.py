"""Spawn a CLI, read its stdout a line at a time, write lines to its stdin.

Every provider adapter talks to its CLI through this. What the lines *mean*
differs per provider; how they move does not.
"""

import subprocess
import threading
from pathlib import Path
from typing import Callable, Sequence


class LineProcess:
    def __init__(
        self,
        argv: Sequence[str],
        on_line: Callable[[str], None],
        env: dict | None = None,
        cwd: str | Path | None = None,
        on_stderr: Callable[[str], None] | None = None,
        on_exit: Callable[[int], None] | None = None,
    ):
        self.argv = list(argv)
        self.on_line = on_line
        self.on_stderr = on_stderr
        self.on_exit = on_exit
        self.env = env
        self.cwd = str(cwd) if cwd else None
        self.proc: subprocess.Popen | None = None
        self.threads: list[threading.Thread] = []
        self.stdin_lock = threading.Lock()

    @property
    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    @property
    def pid(self) -> int | None:
        return self.proc.pid if self.proc else None

    def start(self) -> "LineProcess":
        self.proc = subprocess.Popen(
            self.argv,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.env,
            cwd=self.cwd,
            text=True,
            bufsize=1,
            encoding="utf-8",
            errors="replace",
        )
        self.spawn(self.pump_stdout)
        self.spawn(self.pump_stderr)
        return self

    def spawn(self, target) -> None:
        thread = threading.Thread(target=target, daemon=True)
        thread.start()
        self.threads.append(thread)

    def pump_stdout(self) -> None:
        for line in self.proc.stdout:
            line = line.rstrip("\n")
            if line:
                self.on_line(line)
        code = self.proc.wait()
        if self.on_exit:
            self.on_exit(code)

    def pump_stderr(self) -> None:
        for line in self.proc.stderr:
            if self.on_stderr and line.strip():
                self.on_stderr(line.rstrip("\n"))

    def send_line(self, text: str) -> bool:
        """Write one line to stdin. False means the pipe is gone."""
        if not self.alive:
            return False
        try:
            with self.stdin_lock:
                self.proc.stdin.write(text + "\n")
                self.proc.stdin.flush()
            return True
        except (BrokenPipeError, ValueError, OSError):
            return False

    def close_stdin(self) -> None:
        try:
            with self.stdin_lock:
                if self.proc and self.proc.stdin and not self.proc.stdin.closed:
                    self.proc.stdin.close()
        except OSError:
            pass

    def stop(self, timeout: float = 5.0) -> int | None:
        """EOF first (most CLIs exit cleanly on it), then terminate, then kill.

        The reader threads are joined before returning, so a caller can be sure
        nothing else will arrive through ``on_line`` once this comes back.
        """
        if not self.proc:
            return None
        code = self.reap(timeout)
        for thread in self.threads:
            thread.join(timeout=timeout)
        return code

    def reap(self, timeout: float) -> int | None:
        self.close_stdin()
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            pass
        self.proc.terminate()
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            return self.proc.wait(timeout=timeout)
