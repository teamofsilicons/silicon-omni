"""Build the native daemon and terminal client for platform wheels."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    """Compile the Rust executables immediately before Hatch assembles a wheel."""

    def initialize(self, version: str, build_data: dict[str, Any]) -> None:
        if version != "standard":
            return
        if os.name == "nt":
            raise RuntimeError("silicon-omni wheels currently support macOS and Linux only")

        binaries = self._build_binaries()
        force_include = build_data.setdefault("force_include", {})
        for name, binary in binaries.items():
            force_include[str(binary)] = f"omni/bin/{name}"
        build_data["pure_python"] = False
        build_data["tag"] = self._wheel_tag()

    def _build_binaries(self) -> dict[str, Path]:
        cargo = shutil.which("cargo")
        if cargo is None:
            raise RuntimeError(
                "building silicon-omni requires Cargo and Rust 1.85 or newer; "
                "install Rust from https://rustup.rs"
            )

        command = [
            cargo,
            "build",
            "--release",
            "--locked",
            "--package",
            "omni-daemon",
            "--package",
            "silicon-omni-cli",
            "--bins",
            "--message-format=json-render-diagnostics",
        ]
        result = subprocess.run(
            command,
            cwd=self.root,
            stdout=subprocess.PIPE,
            text=True,
            check=False,
        )

        expected = {"silicon-omni", "so", "omnid"}
        binaries: dict[str, Path] = {}
        diagnostics: list[str] = []
        for line in result.stdout.splitlines():
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue

            if message.get("reason") == "compiler-artifact":
                target = message.get("target", {})
                executable = message.get("executable")
                name = target.get("name")
                if name in expected and executable and "bin" in target.get("kind", []):
                    binaries[name] = Path(executable).resolve()
            elif message.get("reason") == "compiler-message":
                rendered = message.get("message", {}).get("rendered")
                if rendered:
                    diagnostics.append(rendered)

        if diagnostics:
            sys.stderr.write("".join(diagnostics))
        if result.returncode:
            raise RuntimeError(
                "Cargo failed to build silicon-omni/so and omnid "
                f"(exit status {result.returncode})"
            )

        missing = expected.difference(binaries)
        if missing:
            names = ", ".join(sorted(missing))
            raise RuntimeError(
                f"Cargo completed without reporting the built executable(s): {names}"
            )
        for name, binary in binaries.items():
            if not binary.is_file():
                raise RuntimeError(f"Cargo reported a missing {name} executable: {binary}")
            if not os.access(binary, os.X_OK):
                raise RuntimeError(f"Cargo produced a non-executable {name} binary: {binary}")
        return binaries

    @staticmethod
    def _wheel_tag() -> str:
        from packaging.tags import mac_platforms, sys_tags

        for tag in sys_tags():
            platform_tag = tag.platform
            if (
                tag.interpreter == "py3"
                and tag.abi == "none"
                and platform_tag != "any"
                and not platform_tag.startswith(("manylinux", "musllinux"))
            ):
                if sys.platform == "darwin":
                    deployment_target = CustomBuildHook._macos_deployment_target()
                    if deployment_target is not None:
                        _, _, _, architecture = platform_tag.split("_", 3)
                        platform_tag = next(mac_platforms(version=deployment_target, arch=architecture))
                return f"py3-none-{platform_tag}"
        raise RuntimeError("could not determine a native platform wheel tag")

    @staticmethod
    def _macos_deployment_target() -> tuple[int, int] | None:
        rustc = os.environ.get("RUSTC") or shutil.which("rustc")
        if rustc is None:
            return None

        result = subprocess.run(
            [rustc, "--print=deployment-target"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        )
        if result.returncode:
            return None
        try:
            version = result.stdout.strip().rpartition("=")[2]
            major, minor, *_ = (int(part) for part in version.split("."))
        except ValueError:
            return None
        return major, minor
