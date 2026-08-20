import pytest

from omni import providers
from omni.intelligence import registry

from .fake import LIVE, dial, make, rung


@pytest.fixture(autouse=True)
def omni_home(tmp_path, monkeypatch):
    """Every test gets its own ~/.omni, and never reaches the network."""
    monkeypatch.setenv("OMNI_HOME", str(tmp_path / "omni"))
    monkeypatch.setattr(registry, "fetch", lambda name, timeout=5.0: None)
    providers.CACHE.clear()
    LIVE.clear()
    yield
    providers.EXTRA.clear()
    providers.CACHE.clear()


@pytest.fixture
def two_providers():
    """Register 'alpha' (cheap) and 'beta' (strong) and pin their dials."""
    for name in ("alpha", "beta"):
        providers.register(name, *make(name))
    weak = rung("alpha", "alpha-small", "low", 900, 1.0)
    strong = rung("beta", "beta-big", "high", 1800, 9.0)
    registry.write_cache(["alpha", "beta"], dial(strong, weak))
    return ["alpha", "beta"]
