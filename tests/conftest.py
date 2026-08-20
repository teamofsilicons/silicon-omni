import pytest

from omni import providers
from omni.intelligence import registry

from .fake import LIVE, dial, make, rung


@pytest.fixture(autouse=True)
def omni_home(tmp_path, monkeypatch, request):
    """A private ~/.omni per test, and no network.

    Live tests are the exception on purpose: they run against the real home and
    the real registry, because a run that uses different paths from a user's run
    is not testing a user's run. ``scripts/cleanup.py`` takes their leavings out
    again afterwards.
    """
    if not request.node.get_closest_marker("live"):
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
