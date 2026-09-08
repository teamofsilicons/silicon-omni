"""Console-script bridge to the native ``silicon-omni`` terminal client."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import NoReturn


def main() -> NoReturn:
    """Replace this Python process with the terminal client shipped in the wheel."""
    _execute("silicon-omni")


def short() -> NoReturn:
    """Run the same native terminal client through its short name."""
    _execute("so")


def omni() -> NoReturn:
    """The same client again, under the name `omni web` belongs to."""
    _execute("omni")


def _execute(name: str) -> NoReturn:
    binary = Path(__file__).with_name("bin") / name
    try:
        os.execv(str(binary), [str(binary), *sys.argv[1:]])
    except OSError as error:
        raise SystemExit(f"{name}: cannot execute bundled terminal client: {error}") from error
