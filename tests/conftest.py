import pytest

from omni import providers
from omni.intelligence import registry
from omni.providers import test as double


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
    double.LIVE.clear()
    yield
    providers.EXTRA.clear()
    providers.CACHE.clear()
    double.LIVE.clear()


@pytest.fixture
def two_providers():
    """'alpha' (cheap) at the bottom of the dial and 'beta' (strong) at the top."""
    return double.install(
        "beta",
        "alpha",
        rungs=[double.rung("beta", "beta-big", "high"), double.rung("alpha", "alpha-small", "low")],
    )
