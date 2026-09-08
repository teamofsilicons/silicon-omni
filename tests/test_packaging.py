import os
import sys
from pathlib import Path

import pytest

from omni import cli


class Executed(Exception):
    """Stand in for a successful exec, which never returns."""


@pytest.mark.parametrize(
    ("entrypoint", "name"),
    [(cli.main, "silicon-omni"), (cli.short, "so"), (cli.omni, "omni")],
)
def test_console_script_execs_the_bundled_terminal_client(monkeypatch, entrypoint, name):
    seen = {}

    def execute(path, argv):
        seen["path"] = path
        seen["argv"] = argv
        seen["environment"] = os.environ["OMNI_WRAPPER_TEST"]
        raise Executed

    monkeypatch.setattr(os, "execv", execute)
    monkeypatch.setattr(sys, "argv", [f"/a/venv/bin/{name}", "send", "chat", "hello"])
    monkeypatch.setenv("OMNI_WRAPPER_TEST", "preserved")

    with pytest.raises(Executed):
        entrypoint()

    binary = Path(cli.__file__).with_name("bin") / name
    assert seen == {
        "path": str(binary),
        "argv": [str(binary), "send", "chat", "hello"],
        "environment": "preserved",
    }


@pytest.mark.parametrize(
    ("entrypoint", "name"),
    [(cli.main, "silicon-omni"), (cli.short, "so"), (cli.omni, "omni")],
)
def test_console_script_reports_an_exec_failure(monkeypatch, entrypoint, name):
    def fail(path, argv):
        raise OSError("not executable")

    monkeypatch.setattr(os, "execv", fail)

    with pytest.raises(SystemExit, match=rf"^{name}: cannot execute bundled terminal client"):
        entrypoint()
