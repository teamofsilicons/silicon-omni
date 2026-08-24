"""One daemon for the whole run, and a private ``~/.omni`` for it to use.

The daemon is the thing under test, so the tests start a real one and talk to it
over a real socket — there is no in-process shortcut, because there is no
in-process engine any more.

Two details worth knowing if you are adding tests:

* the home lives under ``/tmp`` with a short name. A Unix socket path is capped
  at about a hundred characters, and pytest's own ``tmp_path`` is long enough to
  go over it on a normal checkout.
* every test gets its own session id and its own provider names, so nothing has
  to be torn down between them.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
HOME = Path(f"/tmp/omni-tests-{os.getpid()}")


def built() -> str:
    """Build the current daemon once for this run and return it.

    Cargo's incremental no-op is cheap. Reusing any binary that happens to be
    in ``target`` is not: after a Rust edit it silently tests yesterday's
    daemon against today's Python client.
    """
    subprocess.run(
        ["cargo", "build", "--release", "-p", "omni-daemon"], cwd=ROOT, check=True
    )
    return str(ROOT / "target" / "release" / "omnid")


def wants_live(items) -> bool:
    """Is this a run of the live tests?

    Decide from pytest's selected items, after marker expressions and ignores
    have been applied. A substring test looks fine until somebody writes
    ``--ignore=test_live.py`` or ``-m 'not (live)'``, at which point an offline
    suite can quietly run against the real ``~/.omni``. A mixed selection is
    private too: the real home is used only for an unambiguously all-live run.
    """
    selected = list(items)
    return bool(selected) and all(item.get_closest_marker("live") for item in selected)


@pytest.fixture(scope="session", autouse=True)
def daemon(request):
    """A daemon on a private home, up for the whole run and down after it.

    Live tests are the exception on purpose: they run against the real home and
    the real registry, because a run that uses different paths from a user's run
    is not testing a user's run. ``scripts/cleanup.py`` takes their leavings out
    again afterwards.
    """
    if wants_live(request.session.items):
        yield None
        return
    shutil.rmtree(HOME, ignore_errors=True)
    HOME.mkdir(parents=True)
    os.environ["OMNI_HOME"] = str(HOME)
    os.environ["OMNI_DAEMON"] = built()
    # Nothing in these tests should reach the network for a dial; the double
    # pins its own, and a registry that answered would make the tests depend on
    # what is deployed today.
    os.environ["OMNI_REGISTRY"] = "http://127.0.0.1:1/none"

    from omni.client import daemon as control

    # The belt to the braces above: nothing offline may touch the real home.
    running = control.start()
    real = Path.home() / ".omni"
    assert running.parent.resolve() != real.resolve(), (
        f"refusing to run the offline suite against {real} — "
        "OMNI_HOME did not take effect"
    )
    yield control.version()
    control.stop()
    # Set OMNI_KEEP_HOME to look at what a failing run actually wrote.
    if not os.environ.get("OMNI_KEEP_HOME"):
        shutil.rmtree(HOME, ignore_errors=True)
    else:
        print(f"\nkept the test home at {HOME}")


@pytest.fixture(autouse=True)
def clean_doubles(request):
    """No test provider survives into the next test."""
    if request.node.get_closest_marker("live"):
        yield
        return
    from omni.providers import test as double

    yield
    try:
        double.forget_all()
    except Exception:
        pass


@pytest.fixture
def name(request):
    """A session id nothing else in the run will use."""
    return request.node.name.replace("[", "-").replace("]", "")[:60]


@pytest.fixture
def one(name):
    """A chat over a single test provider, already started."""
    from omni import Inference
    from omni.providers import test as double

    double.install(name)
    chat = Inference.load_or_create_session(name, [name])
    yield chat.start()
    chat.stop()


@pytest.fixture
def pair(name):
    """Two providers: ``<name>-big`` at the top of the dial, ``<name>-small`` at the bottom."""
    from omni import Inference
    from omni.providers import test as double

    big, small = f"{name}-big", f"{name}-small"
    double.install(
        big,
        small,
        rungs=[double.rung(big, "big-model", "high"), double.rung(small, "small-model", "low")],
    )
    chat = Inference.load_or_create_session(name, [big, small])
    yield chat, big, small
    chat.stop()


def settled(chat, timeout: float = 15.0) -> bool:
    """Wait for the chat to have nothing left to do."""
    import time

    deadline = time.time() + timeout
    while time.time() < deadline:
        if chat.idle:
            return True
        time.sleep(0.01)
    return False


def in_turn(chat, timeout: float = 15.0) -> bool:
    """Wait until a message has actually reached a provider."""
    import time

    deadline = time.time() + timeout
    while time.time() < deadline:
        if chat.state.get("in_turn"):
            return True
        time.sleep(0.01)
    return False
