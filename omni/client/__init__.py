"""Talking to the daemon.

Everything omni actually does happens in a Rust daemon; this package is the
whole of how Python reaches it. Two pieces: :mod:`daemon` finds or starts one,
:mod:`link` is the connection.
"""

from .daemon import DaemonError
from .link import Link, call

__all__ = ["Link", "call", "DaemonError"]
