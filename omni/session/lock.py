"""One live chat per session id.

The lock file holds the owning pid. A pid that no longer exists is a dead owner
and its lock is reclaimed: a crashed process must never wedge a session shut.
"""

import atexit
import json
import os
import time

from ..shared import clock, paths


class SessionBusy(RuntimeError):
    """Another live process already owns this session id."""


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


class Lock:
    def __init__(self, session_id: str):
        self.session_id = session_id
        self.path = paths.lock_file(session_id)
        self.held = False

    def acquire(self) -> "Lock":
        paths.ensure(self.path.parent)
        try:
            fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            return self.take_over()
        with os.fdopen(fd, "w") as fh:
            json.dump(self.claim(), fh)
        return self.keep()

    def take_over(self) -> "Lock":
        """Claim a lock whose owner is gone, with exactly one winner.

        Unlinking and re-creating would let two contenders both succeed, so the
        claim is written atomically over the old one and then read back: only
        the process that reads its own pid actually holds it.
        """
        owner = self.owner()
        if owner and alive(owner["pid"]):
            raise SessionBusy(f"session {self.session_id!r} is held by pid {owner['pid']}")
        staging = self.path.with_suffix(f".lock.{os.getpid()}")
        staging.write_text(json.dumps(self.claim()))
        os.replace(staging, self.path)
        for _ in range(2):  # look twice, so a slower contender cannot sneak in behind us
            time.sleep(0.05)
            if (self.owner() or {}).get("pid") != os.getpid():
                raise SessionBusy(f"session {self.session_id!r} was taken by someone else")
        return self.keep()

    def claim(self) -> dict:
        return {"pid": os.getpid(), "at": clock.now()}

    def keep(self) -> "Lock":
        self.held = True
        atexit.register(self.release)
        return self

    def owner(self) -> dict | None:
        try:
            return json.loads(self.path.read_text())
        except (OSError, json.JSONDecodeError):
            return None

    def release(self) -> None:
        if not self.held:
            return
        owner = self.owner()
        if owner and owner.get("pid") == os.getpid():
            self.path.unlink(missing_ok=True)
        self.held = False
