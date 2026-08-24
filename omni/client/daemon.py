"""Finding — and if need be starting — the daemon.

omni's core is a Rust binary that stays running. Nobody should have to know
that: the first thing that wants a session starts one if there is not already
one listening, and every process after that just connects.

The binary is looked for in the order you would want it found: what you said,
what shipped in the wheel, what is on PATH, then a build tree — so a contributor
running from a checkout gets their own build without any setup.
"""

import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

from ..shared import paths

#: How long to wait for a daemon we started to come up.
STARTUP = 15.0


class DaemonError(RuntimeError):
    """The daemon could not be reached, started, or understood."""


def binary() -> str | None:
    """Where omnid is, or None if this machine has not got one."""
    said = os.environ.get("OMNI_DAEMON")
    if said:
        return said if Path(said).is_file() else None
    packaged = Path(__file__).resolve().parent.parent / "bin" / "omnid"
    if packaged.is_file():
        return str(packaged)
    found = shutil.which("omnid")
    if found:
        return found
    return built()


def built() -> str | None:
    """A cargo build in this checkout, so contributors need no install step."""
    for parent in Path(__file__).resolve().parents:
        if not (parent / "Cargo.toml").is_file():
            continue
        for profile in ("release", "debug"):
            candidate = parent / "target" / profile / "omnid"
            if candidate.is_file():
                return str(candidate)
    return None


def listening(path: Path) -> bool:
    """Is somebody actually on the other end, rather than just a file there?"""
    if not path.exists():
        return False
    probe = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        probe.connect(str(path))
        return True
    except OSError:
        return False
    finally:
        probe.close()


def start(timeout: float = STARTUP) -> Path:
    """Make sure a daemon is up, and give back the socket to talk to it on.

    Safe to race: two processes starting one at the same time is normal, and
    the loser finds the winner's socket rather than failing.
    """
    path = paths.socket()
    if listening(path):
        return path
    where = binary()
    if not where:
        raise DaemonError(
            "omni's daemon (omnid) is not installed.\n"
            "  pip install silicon-omni  should bring it; if you are working from a\n"
            "  checkout, run: cargo build --release -p omni-daemon\n"
            "  or point OMNI_DAEMON at the binary."
        )
    paths.home().mkdir(parents=True, exist_ok=True)
    log = open(paths.home() / "omnid.log", "a", encoding="utf-8")
    subprocess.Popen(
        [where],
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=log,
        # Its own session, so the daemon outlives the shell that started it.
        start_new_session=True,
        env=dict(os.environ, OMNI_HOME=str(paths.home())),
    )
    deadline = time.time() + timeout
    while time.time() < deadline:
        if listening(path):
            return path
        time.sleep(0.02)
    raise DaemonError(
        f"started {where} but nothing is listening on {path} after {timeout:g}s.\n"
        f"  Look at {paths.home() / 'omnid.log'} for what it said."
    )


def stop() -> bool:
    """Ask the daemon to shut down. Mostly for tests and `omni daemon stop`."""
    from .link import Link

    try:
        Link.shared().call("shutdown")
    except DaemonError:
        return False
    finally:
        Link.forget()
    for _ in range(100):
        if not listening(paths.socket()):
            return True
        time.sleep(0.05)
    return False


def version() -> dict:
    """What the daemon says about itself. Raises if there is not one."""
    from .link import Link

    return Link.shared().call("ping")


if __name__ == "__main__":  # pragma: no cover - a convenience, not an API
    print(binary() or "omnid not found", file=sys.stderr)
