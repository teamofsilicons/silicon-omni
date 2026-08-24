"""Install-time smoke test run by cibuildwheel outside the source tree."""

from __future__ import annotations

import importlib.metadata
import os
import shutil
import subprocess
from pathlib import Path

import omni


def output(executable: str | Path) -> str:
    print(f"$ {executable} --version")
    result = subprocess.run(
        [str(executable), "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    assert not result.stderr, result.stderr
    value = result.stdout.strip()
    print(value)
    return value


version = importlib.metadata.version("silicon-omni")
terminal = shutil.which("omni")
assert terminal is not None, "the installed wheel did not create the omni console script"

package = Path(omni.__file__).resolve().parent
daemon = package / "bin" / "omnid"
native_terminal = package / "bin" / "omni"
for binary in (native_terminal, daemon):
    assert binary.is_file(), f"the installed wheel is missing {binary.name}"
    assert os.access(binary, os.X_OK), f"the installed {binary.name} is not executable"

assert output(terminal) == f"omni {version}"
assert output(daemon) == f"omnid {version}"
