"""A provider that isn't one.

Just enough behaviour to drive the engine: it records what it was seeded with,
what it was sent, and finishes turns exactly when a test tells it to.
"""

from omni.events import Event
from omni.providers import base

LIVE: dict[str, "FakeRunner"] = {}


def dial(*rungs: dict) -> dict:
    """Spread rungs, best first, over levels 0-10 — what the generator ships."""
    steps = len(rungs) - 1
    return {str(level): rungs[round((10 - level) * steps / 10)] for level in range(11)}


def rung(provider: str, model: str, effort: str = "", score: float = 0, price: float = 0) -> dict:
    return {"provider": provider, "model": model, "effort": effort, "score": score, "price": price}


class FakeAccount(base.Account):
    name = "fake"
    cli = "python3"

    @property
    def installed(self) -> bool:
        return True

    def probe(self) -> str:
        return "authenticated"

    @property
    def limits(self):
        return {"5h": {"used": 0.1, "reset": 0}, "7d": {"used": 0.2, "reset": 0}}


class FakeRunner(base.Runner):
    name = "fake"
    counter = 0

    def start(self, native_id: str = "", history=None) -> None:
        FakeRunner.counter += 1
        if native_id.startswith("gone-"):
            native_id = ""  # the provider no longer knows this session
        self.native_id = native_id or f"{self.name}-{FakeRunner.counter}"
        self.given = list(history or [])
        self.sent: list[str] = []
        self.up = True
        self.resumed = bool(native_id)
        LIVE[self.name] = self

    #: like agy: history is only handed over with the next message
    defer = False

    @property
    def seeded(self) -> bool:
        return not (self.defer and not self.sent)

    def send(self, text: str) -> None:
        self.sent.append(text)
        if self.autoreply:
            self.reply(f"echo:{text}")

    def reply(self, text: str) -> None:
        self.emit(Event(type=Event.TEXT, provider=self.name, model=self.config.model, text=text))
        self.emit(Event(type=Event.END, provider=self.name, model=self.config.model))

    autoreply = True

    def retune(self, model: str, effort: str) -> bool:
        """Like claude and codex: model and effort change without a restart."""
        if not self.tunable:
            return False
        self.config.model, self.config.effort = model, effort
        self.retuned = getattr(self, "retuned", 0) + 1
        return True

    #: agy cannot do this, and says so by returning False
    tunable = True

    def stop(self) -> None:
        self.up = False

    @property
    def alive(self) -> bool:
        return self.up


def make(name: str):
    """A fresh account/runner pair under a different provider name."""
    account = type(f"{name}Account", (FakeAccount,), {"name": name})
    runner = type(f"{name}Runner", (FakeRunner,), {"name": name})
    return account, runner
