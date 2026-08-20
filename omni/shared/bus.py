"""Callback fan-out.

omni is event driven: everything interesting is handed to whoever subscribed.
A handler that raises must never take the run down with it, so failures are
routed to ``on_error`` instead of propagating.
"""

from typing import Callable


class Bus:
    def __init__(self, on_error: Callable | None = None):
        self.handlers: list[Callable] = []
        self.on_error = on_error

    def subscribe(self, fn: Callable) -> Callable:
        """Register a handler. Returns it unchanged, so it works as a decorator."""
        self.handlers.append(fn)
        return fn

    def emit(self, payload) -> None:
        for fn in list(self.handlers):
            try:
                fn(payload)
            except Exception as exc:  # a subscriber's bug is not the run's problem
                if self.on_error:
                    self.on_error(exc, fn, payload)
