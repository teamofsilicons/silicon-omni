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
assert omni.__version__ == version
terminal = shutil.which("silicon-omni")
short_terminal = shutil.which("so")
omni_terminal = shutil.which("omni")
assert terminal is not None, "the installed wheel did not create the silicon-omni script"
assert short_terminal is not None, "the installed wheel did not create the so script"
assert omni_terminal is not None, "the installed wheel did not create the omni script"

package = Path(omni.__file__).resolve().parent
daemon = package / "bin" / "omnid"
native_terminal = package / "bin" / "silicon-omni"
native_short_terminal = package / "bin" / "so"
native_omni_terminal = package / "bin" / "omni"
for binary in (native_terminal, native_short_terminal, native_omni_terminal, daemon):
    assert binary.is_file(), f"the installed wheel is missing {binary.name}"
    assert os.access(binary, os.X_OK), f"the installed {binary.name} is not executable"

assert output(terminal) == f"silicon-omni {version}"
assert output(short_terminal) == f"silicon-omni {version}"
assert output(native_terminal) == f"silicon-omni {version}"
assert output(native_short_terminal) == f"silicon-omni {version}"
assert output(omni_terminal) == f"silicon-omni {version}"
assert output(native_omni_terminal) == f"silicon-omni {version}"
assert output(daemon) == f"omnid {version}"
