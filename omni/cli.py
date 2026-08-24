"""Console-script bridge to the native ``omni`` terminal client."""

from __future__ import annotations

import os
import sys
from pathlib import Path
from typing import NoReturn


def main() -> NoReturn:
    """Replace this Python process with the terminal client shipped in the wheel."""
    binary = Path(__file__).with_name("bin") / "omni"
    try:
        os.execv(str(binary), [str(binary), *sys.argv[1:]])
    except OSError as error:
        raise SystemExit(f"omni: cannot execute bundled terminal client: {error}") from error
