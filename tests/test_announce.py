"""Test group: saying what a provider cannot do, once.

An `unsupported` notice is true of a provider, not of a moment. It belongs on
the two occasions the answer can change — when you ask for the thing, and when
the conversation arrives somewhere new — and nowhere else. Repeating it every
time a process restarts underneath an unchanged conversation is noise, and noise
in an event stream is what makes people stop reading it.
"""

import time

import pytest

from omni.chat import Chat
from omni.events import Event
from omni.providers import test as double

pytestmark = pytest.mark.usefixtures("omni_home")


def settle(chat, want="waiting", timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        if chat.status == want and chat.inbox.empty():
            return True
        time.sleep(0.005)
    return False


def griping(cls):
    """A test provider that cannot honour disable_mcp, the way agy cannot."""

    def announce(self):
        if self.config.disable_mcp:
            self.emit(
                Event(
                    type=Event.CONFIG,
                    provider=self.name,
                    text="unsupported",
                    extra={"ignored": ["disable_mcp"]},
                )
            )

    def start(self, native_id="", history=None):
        announce(self)
        cls.start(self, native_id=native_id, history=history)

    return start


@pytest.fixture
def chat():
    names = double.install(
        "loud",
        "quiet",
        rungs=[double.rung("loud", "loud-big"), double.rung("quiet", "quiet-small")],
    )
    from omni import providers

    for name in names:
        runner = providers.EXTRA[name][1]
        runner.start = griping(double.Runner)
    chat = Chat("announce", names)
    chat.intelligence(10)  # loud owns the top
    yield chat
    chat.stop()


def notices(chat):
    return [e for e in chat.store.events() if e.type == Event.CONFIG and e.text == "unsupported"]


def test_it_is_said_when_the_conversation_arrives(chat):
    chat.start()
    assert settle(chat)
    assert len(notices(chat)) == 1


def test_it_is_not_said_again_every_time_the_process_restarts(chat):
    chat.start()
    assert settle(chat)
    chat.send("one")
    assert settle(chat)
    double.running("loud").fail("crash")  # relaunches the same provider
    assert settle(chat)
    chat.send("two")
    assert settle(chat)
    assert len(notices(chat)) == 1, "the same provider, still unable to do the same thing"


def test_it_is_said_again_when_the_conversation_moves(chat):
    chat.start()
    assert settle(chat)
    chat.intelligence(0)  # down to quiet
    chat.send("move")
    assert settle(chat)
    assert [e.provider for e in notices(chat)] == ["loud", "quiet"]


def test_it_follows_the_setting_rather_than_the_launch(chat):
    """Ask for something this provider can do and it goes quiet; ask again for
    something it cannot and it speaks up again. Two relaunches, one notice each
    time the answer is no."""
    chat.start()
    assert settle(chat)
    assert len(notices(chat)) == 1

    chat.enable_mcp()  # it can honour this — it loads whatever it likes
    chat.send("one")
    assert settle(chat)
    assert len(notices(chat)) == 1, "nothing to complain about now"

    chat.disable_mcp()  # and it cannot honour this
    chat.send("two")
    assert settle(chat)
    assert len(notices(chat)) == 2


def test_coming_back_to_a_provider_hears_it_again(chat):
    """Arriving is the moment worth saying it. Leaving and returning is arriving
    twice, and by then you have been reading another provider's events."""
    chat.start()
    assert settle(chat)
    chat.intelligence(0)
    chat.send("away")
    assert settle(chat)
    chat.intelligence(10)
    chat.send("back")
    assert settle(chat)
    assert [e.provider for e in notices(chat)] == ["loud", "quiet", "loud"]
