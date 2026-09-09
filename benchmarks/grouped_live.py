"""Run the fixed grouped-provider silicon-omni live benchmark.

The workload is deliberately small and rigid:

    Claude: five facts -> Codex: five -> Agy: five -> Claude: recall all 15

It uses only the public Python API shared by silicon-omni 0.3 and later.  One
invocation is one independent trial and writes a self-contained JSON artifact.
Use ``--dry-run`` to inspect the exact transcript without importing omni or
starting any provider.
"""

from __future__ import annotations

import argparse
import importlib
import importlib.metadata
import json
import math
import os
import re
import shlex
import shutil
import statistics
import subprocess
import sys
import threading
import time
import traceback
import uuid
from collections.abc import Iterable
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

# The event boundary and cleanup path intentionally catch provider/library
# exceptions so a failed live trial still emits diagnostics and reaps children.
# ruff: noqa: BLE001


SCHEMA = "silicon-omni.grouped-live/v1"
REPOSITORY_ROOT = Path(__file__).resolve().parents[1]

PROVIDER_LABELS = {
    "claude-code-cli": "Claude",
    "codex-app-server": "Codex",
    "antigravity-cli": "Agy",
}

DEFAULT_MODELS = {
    "claude-code-cli": {"model": "claude-haiku-4-5-20251001", "effort": ""},
    "codex-app-server": {"model": "gpt-6-astra", "effort": "low"},
    "antigravity-cli": {"model": "gemini-3.8-flash-low", "effort": ""},
}

FACTS_BY_PROVIDER = {
    "claude-code-cli": [
        "C01-731482",
        "C02-284617",
        "C03-956843",
        "C04-358426",
        "C05-619275",
    ],
    "codex-app-server": [
        "O01-482965",
        "O02-617358",
        "O03-843731",
        "O04-426284",
        "O05-275956",
    ],
    "antigravity-cli": [
        "G01-965619",
        "G02-358482",
        "G03-731617",
        "G04-284843",
        "G05-956426",
    ],
}

TEACH_TEMPLATE = (
    "Memorize this exact fact for later: {fact}\n"
    "Reply with exactly ACK and nothing else."
)
RECALL_PROMPT = (
    "Return all fifteen fact tokens in the exact chronological order taught. "
    "Output only one compact JSON array of strings with no spaces."
)

ACTIVITY_TYPES = {"thinking", "text", "tool.call", "tool.result"}
TOOL_TYPES = {"tool.call", "tool.result"}
ROUTING_TYPES = {"start", "thinking", "text", "tool.call", "tool.result", "end"}
NON_FATAL_ERROR_KINDS = {"stderr"}
STABLE_NATIVE_KEYS = {
    "native",
    "session",
    "session_id",
    "sessionid",
    "thread",
    "thread_id",
    "threadid",
    "conversation",
    "conversation_id",
    "conversationid",
}


@dataclass(frozen=True)
class TurnSpec:
    label: str
    provider: str
    group_index: int
    prompt: str
    fact: str = ""
    recall: bool = False


class Clock:
    def __init__(self) -> None:
        self.origin_perf_ns = time.perf_counter_ns()
        self.origin_wall_ns = time.time_ns()

    def point(self) -> dict[str, Any]:
        perf_ns = time.perf_counter_ns()
        wall_ns = time.time_ns()
        return {
            "perf_ns": perf_ns,
            "wall_ns": wall_ns,
            "relative_ms": round((perf_ns - self.origin_perf_ns) / 1_000_000, 3),
            "wall": iso_from_ns(wall_ns),
        }

    def relative_ms(self, perf_ns: int | None) -> float | None:
        if perf_ns is None:
            return None
        return round((perf_ns - self.origin_perf_ns) / 1_000_000, 3)


def iso_from_ns(ns: int) -> str:
    return datetime.fromtimestamp(ns / 1_000_000_000, timezone.utc).isoformat()


def json_safe(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        if isinstance(value, float) and not math.isfinite(value):
            return repr(value)
        return value
    if isinstance(value, dict):
        return {str(key): json_safe(item) for key, item in value.items()}
    if isinstance(value, (list, tuple, set)):
        return [json_safe(item) for item in value]
    return repr(value)


def flatten_mapping(value: Any, prefix: str = "") -> Iterable[tuple[str, Any]]:
    if not isinstance(value, dict):
        return
    for key, item in value.items():
        name = f"{prefix}.{key}" if prefix else str(key)
        if isinstance(item, dict):
            yield from flatten_mapping(item, name)
        else:
            yield name, item


def native_values(raw: dict[str, Any]) -> dict[str, str]:
    """Extract only provider conversation/session identities, not turn IDs."""
    found: dict[str, str] = {}
    sources: list[tuple[str, Any]] = [("native", raw.get("native"))]
    extra = raw.get("extra")
    if isinstance(extra, dict):
        sources.append(("extra.native", extra.get("native")))
    for source, value in sources:
        if isinstance(value, dict):
            for path, item in flatten_mapping(value):
                leaf = path.rsplit(".", 1)[-1].lower()
                if leaf in STABLE_NATIVE_KEYS and item not in (None, ""):
                    found[f"{source}.{path}"] = str(item)
        elif value not in (None, ""):
            found[source] = str(value)
    return found


def usage_from_end(raw: dict[str, Any] | None) -> dict[str, Any]:
    extra = (raw or {}).get("extra")
    usage = extra.get("usage") if isinstance(extra, dict) else None
    if not isinstance(usage, dict):
        return {"raw": json_safe(usage), "normalized": {}}

    aliases = {
        "input": {
            "input_tokens",
            "inputtokencount",
            "prompt_tokens",
            "prompttokencount",
        },
        "cached_input": {
            "cache_read_input_tokens",
            "cached_input_tokens",
            "cachedcontenttokencount",
        },
        "cache_creation_input": {
            "cache_creation_input_tokens",
            "cachewriteinputtokens",
        },
        "output": {
            "output_tokens",
            "outputtokencount",
            "completion_tokens",
            "candidatestokencount",
        },
        "reasoning": {
            "reasoning_tokens",
            "reasoningtokens",
            "thinking_tokens",
            "thoughtstokencount",
        },
        "total": {"total", "total_tokens", "totaltokencount"},
    }
    normalized: dict[str, float | int] = {}
    for path, value in flatten_mapping(usage):
        leaf = re.sub(r"[^a-z0-9_]", "", path.rsplit(".", 1)[-1].lower())
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            continue
        for canonical, names in aliases.items():
            if canonical not in normalized and leaf in names:
                normalized[canonical] = value
                break
    return {"raw": json_safe(usage), "normalized": normalized}


def event_dict(event: Any) -> dict[str, Any]:
    try:
        raw = event.to_dict()
    except Exception:
        try:
            raw = vars(event)
        except Exception:
            raw = {"repr": repr(event)}
    return json_safe(dict(raw))


class Recorder:
    """Record callback-observation time and associate events with one active turn."""

    def __init__(self, clock: Clock) -> None:
        self.clock = clock
        self.condition = threading.Condition()
        self.events: list[dict[str, Any]] = []
        self.active: dict[str, Any] | None = None
        self.provider_native: dict[str, dict[str, str]] = {
            provider: {} for provider in PROVIDER_LABELS
        }
        self.callback_failures: list[str] = []

    def callback(self, event: Any) -> None:
        try:
            raw = event_dict(event)
            point = self.clock.point()
            with self.condition:
                index = len(self.events)
                entry = {"index": index, "observed": point, "event": raw}
                self.events.append(entry)
                provider = str(raw.get("provider") or "")
                if provider in self.provider_native:
                    self.provider_native[provider].update(native_values(raw))
                capture = self.active
                if capture is not None:
                    capture["event_indices"].append(index)
                    self._observe(capture, raw, point["perf_ns"], index)
                self.condition.notify_all()
        except Exception as exc:  # the callback itself must never break a run
            with self.condition:
                self.callback_failures.append(repr(exc))
                self.condition.notify_all()

    def _observe(
        self,
        capture: dict[str, Any],
        raw: dict[str, Any],
        observed_ns: int,
        index: int,
    ) -> None:
        event_type = str(raw.get("type") or "")
        provider = str(raw.get("provider") or "")
        extra = raw.get("extra") if isinstance(raw.get("extra"), dict) else {}
        if extra.get("late"):
            capture["late_event_indices"].append(index)
            return

        # CONFIG/SWITCH/NEW_SESSION events may legitimately name both sides of
        # a handoff.  Routing correctness is about the events belonging to the
        # model turn, so do not let bookkeeping make a correct switch fail.
        if provider and event_type in ROUTING_TYPES:
            capture["providers_seen"].add(provider)
        model = str(raw.get("model") or "")
        if provider == capture["provider"] and model and event_type in ROUTING_TYPES:
            capture["models_seen"].add(model)

        after_send = (
            capture.get("send_begin_ns") is not None
            and observed_ns >= capture["send_begin_ns"]
        )
        provider_matches = provider == capture["provider"]
        prompt_matches = str(raw.get("text") or "") == capture["prompt"]

        if (
            event_type == "start"
            and after_send
            and prompt_matches
            and (provider_matches or not provider)
            and capture.get("first_start_ns") is None
        ):
            capture["first_start_ns"] = observed_ns
            capture["start_event_index"] = index

        if after_send and provider_matches and event_type in ACTIVITY_TYPES:
            if capture.get("first_activity_ns") is None:
                capture["first_activity_ns"] = observed_ns
                capture["activity_event_index"] = index
            if event_type == "text" and str(raw.get("text") or ""):
                if capture.get("first_text_ns") is None:
                    capture["first_text_ns"] = observed_ns
                    capture["text_event_index"] = index
                capture["text_parts"].append(str(raw.get("text") or ""))
            if event_type in TOOL_TYPES:
                capture["tool_event_indices"].append(index)

        if event_type == "error" and observed_ns >= capture["begin_ns"]:
            capture["error_event_indices"].append(index)
            if str(raw.get("kind") or "") not in NON_FATAL_ERROR_KINDS:
                capture["fatal_error_indices"].append(index)

        if (
            event_type == "end"
            and after_send
            and (provider_matches or (not provider and capture.get("first_start_ns") is not None))
            and capture.get("end_ns") is None
        ):
            capture["end_ns"] = observed_ns
            capture["end_event_index"] = index
            capture["end_event"] = raw

    def begin_turn(self, spec: TurnSpec) -> dict[str, Any]:
        capture = {
            "label": spec.label,
            "provider": spec.provider,
            "prompt": spec.prompt,
            "begin_ns": time.perf_counter_ns(),
            "begin_wall_ns": time.time_ns(),
            "send_begin_ns": None,
            "send_return_ns": None,
            "first_start_ns": None,
            "first_activity_ns": None,
            "first_text_ns": None,
            "end_ns": None,
            "idle_ns": None,
            "event_indices": [],
            "late_event_indices": [],
            "error_event_indices": [],
            "fatal_error_indices": [],
            "tool_event_indices": [],
            "text_parts": [],
            "providers_seen": set(),
            "models_seen": set(),
            "start_event_index": None,
            "activity_event_index": None,
            "text_event_index": None,
            "end_event_index": None,
            "end_event": None,
        }
        with self.condition:
            if self.active is not None:
                raise RuntimeError("attempted to overlap benchmark turns")
            capture["event_start_index"] = len(self.events)
            self.active = capture
        return capture

    def mark_send(self, capture: dict[str, Any], returning: bool = False) -> int:
        now = time.perf_counter_ns()
        with self.condition:
            key = "send_return_ns" if returning else "send_begin_ns"
            capture[key] = now
            self.condition.notify_all()
        return now

    def finish_turn(self, capture: dict[str, Any]) -> None:
        with self.condition:
            capture["event_end_index"] = len(self.events)
            if self.active is capture:
                self.active = None

    def wait_for_end(self, capture: dict[str, Any], chat: Any, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while True:
            with self.condition:
                if capture.get("end_ns") is not None:
                    return
                callback_failure = self.callback_failures[-1] if self.callback_failures else ""
                fatal = bool(capture["fatal_error_indices"])
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError(
                        f"{capture['label']} saw no matching END within {timeout:g}s"
                    )
                self.condition.wait(min(remaining, 0.1))
            if callback_failure:
                raise RuntimeError(f"event recorder failed: {callback_failure}")
            if fatal and bool(getattr(chat, "idle", False)):
                raise RuntimeError(
                    f"{capture['label']} became idle after a fatal ERROR without END"
                )


def classify_command(command: str) -> str:
    lowered = command.lower()
    try:
        tokens = shlex.split(command)
    except ValueError:
        tokens = command.split()
    bases = [Path(token).name.lower() for token in tokens[:4]]
    if any(base == "omnid" for base in bases) or re.search(r"(?:^|/)omnid(?:\s|$)", lowered):
        return "daemon"
    if re.search(r"(?:^|[/\s])codex\s+app-server(?:\s|$)", lowered):
        return "codex-app-server"
    if (
        any(base == "claude" for base in bases)
        or re.search(r"(?:^|[/\s])claude(?:\s|$)", lowered)
        or "@anthropic-ai/claude-code" in lowered
        or "/claude-code/" in lowered
    ):
        return "claude-code-cli"
    if any(base == "agy" for base in bases) or re.search(r"(?:^|[/\s])agy(?:\s|$)", lowered):
        return "antigravity-cli"
    return ""


class ProcessTracker:
    """Poll descendants of this harness and retain only provider/daemon roots."""

    def __init__(self, clock: Clock, interval_ms: int) -> None:
        self.clock = clock
        self.root_pid = os.getpid()
        self.interval = max(interval_ms, 20) / 1000
        self.lock = threading.Lock()
        self.stop_event = threading.Event()
        self.thread: threading.Thread | None = None
        self.timeline: list[dict[str, Any]] = []
        self.marks: list[dict[str, Any]] = []
        self.known: dict[str, dict[str, Any]] = {}
        self.errors: list[str] = []
        self.last_signature: tuple = ()

    def start(self) -> None:
        self.thread = threading.Thread(
            target=self._poll, daemon=True, name="benchmark:process-tracker"
        )
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()
        if self.thread is not None:
            self.thread.join(timeout=max(1.0, self.interval * 4))

    def _poll(self) -> None:
        while not self.stop_event.is_set():
            try:
                self._capture("poll", mark=False)
            except Exception as exc:
                with self.lock:
                    self.errors.append(repr(exc))
            self.stop_event.wait(self.interval)

    @staticmethod
    def _rows() -> dict[int, dict[str, Any]]:
        done = subprocess.run(
            ["ps", "-axo", "pid=,ppid=,lstart=,state=,command="],
            capture_output=True,
            text=True,
            timeout=5,
            check=True,
        )
        rows: dict[int, dict[str, Any]] = {}
        for line in done.stdout.splitlines():
            parts = line.strip().split(None, 8)
            if len(parts) < 9:
                continue
            try:
                pid, ppid = int(parts[0]), int(parts[1])
            except ValueError:
                continue
            rows[pid] = {
                "pid": pid,
                "ppid": ppid,
                "started": " ".join(parts[2:7]),
                "state": parts[7],
                "command": parts[8],
            }
        return rows

    def _scoped(self, rows: dict[int, dict[str, Any]]) -> dict[int, dict[str, Any]]:
        pids = {self.root_pid}
        changed = True
        while changed:
            changed = False
            for pid, row in rows.items():
                if pid not in pids and row["ppid"] in pids:
                    pids.add(pid)
                    changed = True
        return {pid: rows[pid] for pid in pids if pid in rows and pid != self.root_pid}

    @staticmethod
    def _roots(scoped: dict[int, dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
        classified = {
            pid: classify_command(row["command"]) for pid, row in scoped.items()
        }
        grouped: dict[str, list[dict[str, Any]]] = {
            "claude-code-cli": [],
            "codex-app-server": [],
            "antigravity-cli": [],
            "daemon": [],
        }
        for pid, kind in classified.items():
            if not kind:
                continue
            row = scoped[pid]
            if classified.get(row["ppid"]) == kind:
                continue
            grouped[kind].append(row)
        for rows in grouped.values():
            rows.sort(key=lambda row: row["pid"])
        return grouped

    def _capture(self, phase: str, mark: bool) -> dict[str, Any]:
        rows = self._rows()
        grouped = self._roots(self._scoped(rows))
        point = self.clock.point()
        compact = {
            kind: [json_safe(row) for row in provider_rows]
            for kind, provider_rows in grouped.items()
        }
        signature = tuple(
            (kind, row["pid"], row["started"])
            for kind in sorted(compact)
            for row in compact[kind]
        )
        record = {"phase": phase, "observed": point, "processes": compact}
        with self.lock:
            for kind, provider_rows in compact.items():
                for row in provider_rows:
                    identity = f"{row['pid']}:{row['started']}"
                    previous = self.known.get(identity)
                    if previous is None:
                        self.known[identity] = {
                            **row,
                            "kind": kind,
                            "first_seen_ms": point["relative_ms"],
                            "last_seen_ms": point["relative_ms"],
                            "observations": 1,
                        }
                    else:
                        previous["last_seen_ms"] = point["relative_ms"]
                        previous["observations"] += 1
            if signature != self.last_signature:
                self.timeline.append(record)
                self.last_signature = signature
            if mark:
                self.marks.append(record)
        return record

    def mark(self, phase: str) -> dict[str, Any]:
        try:
            return self._capture(phase, mark=True)
        except Exception as exc:
            with self.lock:
                self.errors.append(repr(exc))
            fallback = {
                "phase": phase,
                "observed": self.clock.point(),
                "processes": {kind: [] for kind in (*PROVIDER_LABELS, "daemon")},
                "error": repr(exc),
            }
            with self.lock:
                self.marks.append(fallback)
            return fallback

    def alive_known(self) -> list[dict[str, Any]]:
        try:
            rows = self._rows()
        except Exception as exc:
            with self.lock:
                self.errors.append(repr(exc))
            return []
        with self.lock:
            known = list(self.known.values())
        alive = []
        for item in known:
            row = rows.get(int(item["pid"]))
            if row and row["started"] == item["started"]:
                alive.append({**json_safe(row), "kind": item["kind"]})
        return alive

    def report(self) -> dict[str, Any]:
        with self.lock:
            return {
                "root_pid": self.root_pid,
                "poll_interval_ms": round(self.interval * 1000, 3),
                "known_processes": sorted(
                    (json_safe(item) for item in self.known.values()),
                    key=lambda item: (item["kind"], item["first_seen_ms"], item["pid"]),
                ),
                "timeline": json_safe(self.timeline),
                "marks": json_safe(self.marks),
                "errors": list(self.errors),
            }


def all_facts() -> list[str]:
    return [
        fact
        for provider in ("claude-code-cli", "codex-app-server", "antigravity-cli")
        for fact in FACTS_BY_PROVIDER[provider]
    ]


def workload() -> list[TurnSpec]:
    turns: list[TurnSpec] = []
    prefixes = {"claude-code-cli": "C", "codex-app-server": "O", "antigravity-cli": "G"}
    for provider in ("claude-code-cli", "codex-app-server", "antigravity-cli"):
        for index, fact in enumerate(FACTS_BY_PROVIDER[provider], start=1):
            turns.append(
                TurnSpec(
                    label=f"{prefixes[provider]}{index}",
                    provider=provider,
                    group_index=index,
                    prompt=TEACH_TEMPLATE.format(fact=fact),
                    fact=fact,
                )
            )
    turns.append(
        TurnSpec(
            label="RECALL",
            provider="claude-code-cli",
            group_index=1,
            prompt=RECALL_PROMPT,
            recall=True,
        )
    )
    return turns


def models_from_args(args: argparse.Namespace) -> dict[str, dict[str, str]]:
    return {
        "claude-code-cli": {"model": args.claude_model, "effort": args.claude_effort},
        "codex-app-server": {"model": args.codex_model, "effort": args.codex_effort},
        "antigravity-cli": {"model": args.agy_model, "effort": args.agy_effort},
    }


def benchmark_plan(args: argparse.Namespace) -> dict[str, Any]:
    facts = all_facts()
    expected = json.dumps(facts, separators=(",", ":"))
    return {
        "providers": ["claude-code-cli", "codex-app-server", "antigravity-cli", "claude-code-cli"],
        "facts": facts,
        "expected_recall": expected,
        "models": models_from_args(args),
        "level": 0,
        "turns": [
            {
                "label": spec.label,
                "provider": spec.provider,
                "provider_label": PROVIDER_LABELS[spec.provider],
                "group_index": spec.group_index,
                "fact": spec.fact,
                "recall": spec.recall,
                "prompt": spec.prompt,
            }
            for spec in workload()
        ],
    }


def ensure_fresh_home(path: Path, reuse: bool) -> None:
    forbidden = {Path("/").resolve(), Path.home().resolve(), REPOSITORY_ROOT.resolve()}
    resolved = path.resolve()
    if resolved in forbidden:
        raise ValueError(f"refusing unsafe OMNI_HOME: {resolved}")
    if path.exists() and not path.is_dir():
        raise ValueError(f"OMNI_HOME is not a directory: {path}")
    if path.exists() and any(path.iterdir()) and not reuse:
        raise ValueError(
            f"OMNI_HOME is not empty: {path}; use a fresh directory "
            "(--reuse-home makes the trial non-independent)"
        )
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(path, 0o700)


def write_dial(home: Path, models: dict[str, dict[str, str]]) -> Path:
    cache_dir = home / "cache"
    cache_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    now = time.time()
    blob: dict[str, Any] = {"version": 2}
    for provider, chosen in models.items():
        blob[provider] = {
            "at": now,
            "ttl": 10 * 365 * 24 * 60 * 60,
            "levels": {
                "0": {
                    "provider": provider,
                    "model": chosen["model"],
                    "effort": chosen["effort"],
                }
            },
        }
    target = cache_dir / "intelligence.json"
    target.write_text(json.dumps(blob, separators=(",", ":")), encoding="utf-8")
    os.chmod(target, 0o600)
    return target


def is_within(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def import_and_assert_omni(args: argparse.Namespace) -> tuple[Any, Any, dict[str, Any]]:
    omni = importlib.import_module("omni")
    module_file = Path(omni.__file__).resolve()
    package_version = str(getattr(omni, "__version__", ""))
    distribution_version = importlib.metadata.version("silicon-omni")
    if package_version != args.expected_version:
        raise AssertionError(
            f"omni.__version__ is {package_version!r}, expected {args.expected_version!r}"
        )
    if distribution_version != args.expected_version:
        raise AssertionError(
            f"installed silicon-omni is {distribution_version!r}, "
            f"expected {args.expected_version!r}"
        )
    if not args.allow_source_tree and is_within(module_file, REPOSITORY_ROOT):
        raise AssertionError(
            f"imported omni from the checkout ({module_file}); install the target wheel "
            "in an isolated venv or pass --allow-source-tree deliberately"
        )
    if args.expected_module_root:
        wanted = Path(args.expected_module_root).resolve()
        if not is_within(module_file, wanted):
            raise AssertionError(f"omni module {module_file} is not under {wanted}")
    Inference = omni.Inference
    architecture = "daemon" if callable(getattr(Inference, "daemon", None)) else "in-process"
    expected_architecture = args.expect_architecture
    if expected_architecture != "auto" and architecture != expected_architecture:
        raise AssertionError(
            f"detected {architecture} architecture, expected {expected_architecture}"
        )
    module_paths = {"omni": str(module_file)}
    for name in ("omni.chat", "omni.inference", "omni.events"):
        module = importlib.import_module(name)
        module_paths[name] = str(Path(module.__file__).resolve())
    return omni, Inference, {
        "package_version": package_version,
        "distribution_version": distribution_version,
        "architecture": architecture,
        "module_paths": module_paths,
    }


def assert_daemon(
    Inference: Any,
    args: argparse.Namespace,
    omni_home: Path,
) -> dict[str, Any]:
    info = json_safe(Inference.daemon())
    expected = args.expected_daemon_version or args.expected_version
    if str(info.get("version") or "") != expected:
        raise AssertionError(
            f"daemon version is {info.get('version')!r}, expected {expected!r}"
        )
    daemon_home = info.get("home")
    if daemon_home and Path(str(daemon_home)).resolve() != omni_home.resolve():
        raise AssertionError(
            f"daemon home is {daemon_home!r}, expected {str(omni_home)!r}"
        )
    if not isinstance(info.get("pid"), int):
        raise TypeError(f"daemon did not report an integer pid: {info!r}")
    return info


def wait_idle(chat: Any, timeout: float, label: str) -> int:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if bool(getattr(chat, "idle", False)):
            return time.perf_counter_ns()
        time.sleep(0.01)
    raise TimeoutError(f"{label} did not become idle within {timeout:g}s")


def ms_between(start: int | None, end: int | None) -> float | None:
    if start is None or end is None:
        return None
    return round((end - start) / 1_000_000, 3)


def event_at(recorder: Recorder, index: int | None) -> dict[str, Any] | None:
    if index is None:
        return None
    try:
        return recorder.events[index]["event"]
    except (IndexError, TypeError):
        return None


def process_pids(mark: dict[str, Any], provider: str) -> list[int]:
    processes = mark.get("processes") if isinstance(mark, dict) else {}
    rows = processes.get(provider, []) if isinstance(processes, dict) else []
    return sorted(int(row["pid"]) for row in rows if isinstance(row.get("pid"), int))


def build_turn_result(
    spec: TurnSpec,
    capture: dict[str, Any],
    recorder: Recorder,
    clock: Clock,
    expected_model: str,
    config_begin_ns: int | None,
    config_return_ns: int | None,
    before_processes: dict[str, Any],
    after_processes: dict[str, Any],
) -> dict[str, Any]:
    output = "".join(capture["text_parts"])
    errors = [event_at(recorder, index) for index in capture["error_event_indices"]]
    errors = [error for error in errors if error is not None]
    end_event = capture.get("end_event")
    anchors = {
        "turn_begin_ms": clock.relative_ms(capture.get("begin_ns")),
        "configure_begin_ms": clock.relative_ms(config_begin_ns),
        "configure_return_ms": clock.relative_ms(config_return_ns),
        "send_begin_ms": clock.relative_ms(capture.get("send_begin_ns")),
        "send_return_ms": clock.relative_ms(capture.get("send_return_ns")),
        "start_observed_ms": clock.relative_ms(capture.get("first_start_ns")),
        "activity_observed_ms": clock.relative_ms(capture.get("first_activity_ns")),
        "text_observed_ms": clock.relative_ms(capture.get("first_text_ns")),
        "end_observed_ms": clock.relative_ms(capture.get("end_ns")),
        "idle_observed_ms": clock.relative_ms(capture.get("idle_ns")),
    }
    timings = {
        "configure_call_ms": ms_between(config_begin_ns, config_return_ns),
        "send_call_ms": ms_between(capture.get("send_begin_ns"), capture.get("send_return_ns")),
        "send_to_start_ms": ms_between(capture.get("send_begin_ns"), capture.get("first_start_ns")),
        "send_to_activity_ms": ms_between(
            capture.get("send_begin_ns"), capture.get("first_activity_ns")
        ),
        "send_to_text_ms": ms_between(capture.get("send_begin_ns"), capture.get("first_text_ns")),
        "send_to_end_ms": ms_between(capture.get("send_begin_ns"), capture.get("end_ns")),
        "end_to_idle_ms": ms_between(capture.get("end_ns"), capture.get("idle_ns")),
        "turn_total_ms": ms_between(capture.get("begin_ns"), capture.get("idle_ns")),
        "switch_to_end_ms": ms_between(config_begin_ns, capture.get("end_ns")),
    }
    providers_seen = sorted(capture["providers_seen"])
    models_seen = sorted(capture["models_seen"])
    return {
        "label": spec.label,
        "provider": spec.provider,
        "provider_label": PROVIDER_LABELS[spec.provider],
        "group_index": spec.group_index,
        "fact": spec.fact,
        "recall": spec.recall,
        "prompt": spec.prompt,
        "output": output,
        "anchors": anchors,
        "timings": timings,
        "event_indices": list(capture["event_indices"]),
        "late_event_indices": list(capture["late_event_indices"]),
        "first_events": {
            "start": event_at(recorder, capture.get("start_event_index")),
            "activity": event_at(recorder, capture.get("activity_event_index")),
            "text": event_at(recorder, capture.get("text_event_index")),
            "end": end_event,
        },
        "event_at": {
            "start": (event_at(recorder, capture.get("start_event_index")) or {}).get("at"),
            "activity": (event_at(recorder, capture.get("activity_event_index")) or {}).get("at"),
            "text": (event_at(recorder, capture.get("text_event_index")) or {}).get("at"),
            "end": (end_event or {}).get("at"),
        },
        "routing": {
            "expected_provider": spec.provider,
            "providers_seen": providers_seen,
            "expected_model": expected_model,
            "models_seen": models_seen,
            "provider_ok": spec.provider in providers_seen
            and not any(provider != spec.provider for provider in providers_seen),
            "model_ok": expected_model in models_seen
            and not any(model != expected_model for model in models_seen),
        },
        "errors": errors,
        "fatal_error_count": len(capture["fatal_error_indices"]),
        "tool_event_count": len(capture["tool_event_indices"]),
        "usage": usage_from_end(end_event),
        "native": {
            "observed_in_turn": {
                key: value
                for index in capture["event_indices"]
                for key, value in native_values(recorder.events[index]["event"]).items()
            },
            "known_after_turn": dict(recorder.provider_native[spec.provider]),
        },
        "processes": {
            "before": before_processes,
            "after": after_processes,
            "provider_pids_before": process_pids(before_processes, spec.provider),
            "provider_pids_after": process_pids(after_processes, spec.provider),
        },
        "complete": capture.get("end_ns") is not None and capture.get("idle_ns") is not None,
    }


def run_turn(
    chat: Any,
    spec: TurnSpec,
    recorder: Recorder,
    processes: ProcessTracker,
    clock: Clock,
    args: argparse.Namespace,
    expected_model: str,
    active_provider: str,
) -> tuple[dict[str, Any], str, BaseException | None]:
    capture = recorder.begin_turn(spec)
    before = processes.mark(f"{spec.label}:before")
    config_begin_ns = None
    config_return_ns = None
    after: dict[str, Any] | None = None
    turn_failure: BaseException | None = None
    try:
        if active_provider != spec.provider:
            config_begin_ns = time.perf_counter_ns()
            chat.active_inference_providers([spec.provider])
            config_return_ns = time.perf_counter_ns()
            active_provider = spec.provider
        recorder.mark_send(capture)
        chat.send(spec.prompt)
        recorder.mark_send(capture, returning=True)
        recorder.wait_for_end(capture, chat, args.turn_timeout)
        capture["idle_ns"] = wait_idle(chat, args.idle_timeout, spec.label)
        # Let the process poller observe the settled boundary as well.
        if args.settle_ms:
            time.sleep(args.settle_ms / 1000)
        after = processes.mark(f"{spec.label}:after_idle")
    except BaseException as exc:
        turn_failure = exc
        if bool(getattr(chat, "idle", False)):
            capture["idle_ns"] = time.perf_counter_ns()
        after = processes.mark(f"{spec.label}:after_failure")
    finally:
        if after is None:
            after = processes.mark(f"{spec.label}:after_failure")
        result = build_turn_result(
            spec,
            capture,
            recorder,
            clock,
            expected_model,
            config_begin_ns,
            config_return_ns,
            before,
            after,
        )
        if turn_failure is not None:
            result["failure"] = {
                "type": type(turn_failure).__name__,
                "message": str(turn_failure),
            }
        recorder.finish_turn(capture)
    return result, active_provider, turn_failure


def parsed_recall(text: str) -> Any:
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def median(values: Iterable[float | None]) -> float | None:
    present = [value for value in values if isinstance(value, (int, float))]
    return round(statistics.median(present), 3) if present else None


def total(values: Iterable[float | None]) -> float | None:
    present = [value for value in values if isinstance(value, (int, float))]
    return round(sum(present), 3) if present else None


def group_summary(provider: str, turns: list[dict[str, Any]]) -> dict[str, Any]:
    first = turns[0] if turns else None
    steady = turns[1:]
    pid_sets = [tuple(turn["processes"]["provider_pids_after"]) for turn in turns]
    nonempty_pid_sets = [pid_set for pid_set in pid_sets if pid_set]
    first_end = first["anchors"]["end_observed_ms"] if first else None
    last_idle = turns[-1]["anchors"]["idle_observed_ms"] if turns else None
    first_begin = first["anchors"]["turn_begin_ms"] if first else None
    group_total = (
        round(last_idle - first_begin, 3)
        if isinstance(last_idle, (int, float)) and isinstance(first_begin, (int, float))
        else None
    )
    return {
        "provider": provider,
        "provider_label": PROVIDER_LABELS[provider],
        "turn_labels": [turn["label"] for turn in turns],
        "first_turn_send_to_end_ms": (
            first["timings"]["send_to_end_ms"] if first else None
        ),
        "switch_to_first_end_ms": (
            first["timings"]["switch_to_end_ms"] if first else None
        ),
        "steady_turns_2_to_5": {
            "send_to_activity_median_ms": median(
                turn["timings"]["send_to_activity_ms"] for turn in steady
            ),
            "send_to_text_median_ms": median(
                turn["timings"]["send_to_text_ms"] for turn in steady
            ),
            "send_to_end_median_ms": median(
                turn["timings"]["send_to_end_ms"] for turn in steady
            ),
            "send_to_end_sum_ms": total(
                turn["timings"]["send_to_end_ms"] for turn in steady
            ),
        },
        "activation_premium_vs_steady_median_ms": (
            round(
                first["timings"]["send_to_end_ms"]
                - statistics.median(
                    turn["timings"]["send_to_end_ms"]
                    for turn in steady
                    if isinstance(turn["timings"]["send_to_end_ms"], (int, float))
                ),
                3,
            )
            if first
            and isinstance(first["timings"]["send_to_end_ms"], (int, float))
            and any(
                isinstance(turn["timings"]["send_to_end_ms"], (int, float))
                for turn in steady
            )
            else None
        ),
        "group_total_ms": group_total,
        "first_end_observed_ms": first_end,
        "provider_pid_sets_after_turns": [list(pid_set) for pid_set in pid_sets],
        "provider_pid_set_stable": bool(nonempty_pid_sets)
        and all(pid_set == nonempty_pid_sets[0] for pid_set in nonempty_pid_sets)
        and len(nonempty_pid_sets) == len(pid_sets),
        "native_known_after_turns": [turn["native"]["known_after_turn"] for turn in turns],
    }


def summarize(result: dict[str, Any]) -> dict[str, Any]:
    turns = result.get("turns", [])
    facts = all_facts()
    expected_raw = json.dumps(facts, separators=(",", ":"))
    teaching = [turn for turn in turns if not turn.get("recall")]
    recall = next((turn for turn in turns if turn.get("recall")), None)
    ack_results = [
        {
            "label": turn["label"],
            "raw_exact": turn.get("output") == "ACK",
            "trimmed_exact": str(turn.get("output") or "").strip() == "ACK",
            "output": turn.get("output"),
        }
        for turn in teaching
    ]
    recall_text = str(recall.get("output") or "") if recall else ""
    parsed = parsed_recall(recall_text)
    positions = [recall_text.find(fact) for fact in facts]
    all_present = all(position >= 0 for position in positions)
    ordered = all_present and positions == sorted(positions)
    recall_check = {
        "expected_raw": expected_raw,
        "actual_raw": recall_text,
        "json_parsed": parsed,
        "json_array_exact": parsed == facts,
        "raw_exact": recall_text == expected_raw,
        "facts_present": sum(fact in recall_text for fact in facts),
        "all_facts_present": all_present,
        "facts_in_order": ordered,
        "each_fact_once": all(recall_text.count(fact) == 1 for fact in facts),
    }
    groups = []
    for provider in ("claude-code-cli", "codex-app-server", "antigravity-cli"):
        groups.append(group_summary(provider, [turn for turn in teaching if turn["provider"] == provider]))

    provider_processes_observed = all(
        any(
            turn["processes"]["provider_pids_after"]
            for turn in teaching
            if turn["provider"] == provider
        )
        for provider in PROVIDER_LABELS
    )
    native_identity_observed = all(
        any(
            turn["native"]["known_after_turn"]
            for turn in teaching
            if turn["provider"] == provider
        )
        for provider in PROVIDER_LABELS
    )
    instrumentation_clean = not result.get("callback_failures") and not result.get(
        "process_tracking", {}
    ).get("errors")

    startup = result.get("startup", {})
    first_turn = teaching[0] if teaching else None
    final_idle = recall["anchors"]["idle_observed_ms"] if recall else None
    start_begin = startup.get("start_begin_ms")
    start_to_first_end = None
    workflow_ms = None
    if first_turn and isinstance(start_begin, (int, float)):
        end = first_turn["anchors"]["end_observed_ms"]
        if isinstance(end, (int, float)):
            start_to_first_end = round(end - start_begin, 3)
    if isinstance(start_begin, (int, float)) and isinstance(final_idle, (int, float)):
        workflow_ms = round(final_idle - start_begin, 3)

    routing_ok = len(turns) == 16 and all(
        turn["routing"]["provider_ok"] and turn["routing"]["model_ok"] for turn in turns
    )
    no_fatal = len(turns) == 16 and all(turn["fatal_error_count"] == 0 for turn in turns)
    no_tools = len(turns) == 16 and all(turn["tool_event_count"] == 0 for turn in turns)
    complete = len(turns) == 16 and all(turn["complete"] for turn in turns)
    ack_exact = len(ack_results) == 15 and all(item["raw_exact"] for item in ack_results)
    cleanup_ok = bool(result.get("cleanup", {}).get("clean"))
    overall = {
        "turn_count": len(turns),
        "all_turns_complete": complete,
        "routing_exact": routing_ok,
        "all_teaching_ack_raw_exact": ack_exact,
        "no_fatal_errors": no_fatal,
        "no_tools": no_tools,
        "recall_json_array_exact": recall_check["json_array_exact"],
        "recall_raw_exact": recall_check["raw_exact"],
        "cleanup_clean": cleanup_ok,
        "instrumentation_clean": instrumentation_clean,
        "provider_processes_observed": provider_processes_observed,
        "native_identities_observed": native_identity_observed,
        "provider_pids_stable_within_groups": all(
            group["provider_pid_set_stable"] for group in groups
        ),
    }
    overall["pass"] = all(overall.values())
    initial_claude = next(
        (turn for turn in teaching if turn["provider"] == "claude-code-cli"),
        {"native": {"known_after_turn": {}}, "processes": {"provider_pids_after": []}},
    )
    initial_native = initial_claude["native"]["known_after_turn"]
    recall_native = recall["native"]["known_after_turn"] if recall else {}
    initial_native_values = set(initial_native.values())
    recall_native_values = set(recall_native.values())
    initial_pids = initial_claude["processes"]["provider_pids_after"]
    recall_pids = recall["processes"]["provider_pids_after"] if recall else []
    return {
        "speed": {
            "start_to_first_claude_end_ms": start_to_first_end,
            "workflow_start_to_recall_idle_ms": workflow_ms,
            "teaching_send_to_end_sum_ms": total(
                turn["timings"]["send_to_end_ms"] for turn in teaching
            ),
            "teaching_send_to_end_median_ms": median(
                turn["timings"]["send_to_end_ms"] for turn in teaching
            ),
            "recall_send_to_activity_ms": (
                recall["timings"]["send_to_activity_ms"] if recall else None
            ),
            "recall_send_to_text_ms": recall["timings"]["send_to_text_ms"] if recall else None,
            "recall_send_to_end_ms": recall["timings"]["send_to_end_ms"] if recall else None,
            "groups": groups,
        },
        "ack_checks": ack_results,
        "recall": recall_check,
        "overall": overall,
        "claude_return": {
            "initial_native": initial_native,
            "recall_native": recall_native,
            "native_value_overlap": sorted(initial_native_values & recall_native_values),
            "native_continuity_observed": bool(initial_native_values & recall_native_values),
            "initial_provider_pids": initial_pids,
            "recall_provider_pids": recall_pids,
            "same_provider_pid_set": bool(initial_pids) and initial_pids == recall_pids,
        },
    }


def write_result(path: str, result: dict[str, Any]) -> None:
    payload = json.dumps(json_safe(result), indent=2, sort_keys=True) + "\n"
    if path == "-":
        sys.stdout.write(payload)
        return
    target = Path(path).resolve()
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_name(f".{target.name}.tmp.{os.getpid()}")
    temporary.write_text(payload, encoding="utf-8")
    os.replace(temporary, target)


def probe_cli_versions(paths: dict[str, str | None]) -> dict[str, Any]:
    """Capture CLI builds after the timed workload, so probing cannot pre-warm it."""
    versions: dict[str, Any] = {}
    for name, path in paths.items():
        if not path:
            versions[name] = {"path": None, "error": "not found"}
            continue
        begin = time.perf_counter_ns()
        try:
            done = subprocess.run(
                [path, "--version"],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            versions[name] = {
                "path": path,
                "returncode": done.returncode,
                "stdout": done.stdout.strip()[:4096],
                "stderr": done.stderr.strip()[:4096],
                "duration_ms": ms_between(begin, time.perf_counter_ns()),
            }
        except Exception as exc:
            versions[name] = {
                "path": path,
                "error": repr(exc),
                "duration_ms": ms_between(begin, time.perf_counter_ns()),
            }
    return versions


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Benchmark one silicon-omni Python build with Claude 5 -> Codex 5 -> "
            "Agy 5 -> Claude recall. One invocation is one fresh trial."
        ),
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )
    parser.add_argument("--expected-version", required=True, help="exact omni and wheel version")
    parser.add_argument(
        "--expect-architecture",
        choices=("auto", "in-process", "daemon"),
        default="auto",
        help="assert the expected Python runtime architecture",
    )
    parser.add_argument(
        "--expected-daemon-version",
        help="exact daemon version; defaults to --expected-version",
    )
    parser.add_argument("--expected-module-root", help="assert omni imports below this path")
    parser.add_argument(
        "--allow-source-tree",
        action="store_true",
        help="allow importing omni directly from this checkout (not release-comparison safe)",
    )
    parser.add_argument("--omni-home", required=True, help="fresh, dedicated OMNI_HOME")
    parser.add_argument("--workdir", required=True, help="neutral provider working directory")
    parser.add_argument("--output", required=True, help="result JSON path, or - for stdout")
    parser.add_argument("--session-id", help="unique omni session id")
    parser.add_argument("--trial", default="", help="free-form trial/order label stored in JSON")
    parser.add_argument(
        "--reuse-home",
        action="store_true",
        help="permit a nonempty OMNI_HOME (marks the trial as non-independent)",
    )
    parser.add_argument(
        "--preflight",
        action="store_true",
        help="probe provider authentication before timing (changes the startup state)",
    )
    parser.add_argument(
        "--skip-cli-check",
        action="store_true",
        help="skip the non-invasive PATH check for claude, codex, and agy",
    )
    parser.add_argument("--turn-timeout", type=float, default=300.0)
    parser.add_argument("--idle-timeout", type=float, default=30.0)
    parser.add_argument("--cleanup-timeout", type=float, default=100.0)
    parser.add_argument("--process-poll-ms", type=int, default=100)
    parser.add_argument(
        "--settle-ms",
        type=int,
        default=50,
        help="post-idle observation grace before the process snapshot",
    )
    parser.add_argument("--claude-model", default=DEFAULT_MODELS["claude-code-cli"]["model"])
    parser.add_argument("--claude-effort", default=DEFAULT_MODELS["claude-code-cli"]["effort"])
    parser.add_argument("--codex-model", default=DEFAULT_MODELS["codex-app-server"]["model"])
    parser.add_argument("--codex-effort", default=DEFAULT_MODELS["codex-app-server"]["effort"])
    parser.add_argument("--agy-model", default=DEFAULT_MODELS["antigravity-cli"]["model"])
    parser.add_argument("--agy-effort", default=DEFAULT_MODELS["antigravity-cli"]["effort"])
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="write/print only the fixed plan; do not import omni or call providers",
    )
    return parser


def execute(args: argparse.Namespace) -> dict[str, Any]:
    clock = Clock()
    plan = benchmark_plan(args)
    omni_home = Path(args.omni_home).expanduser().resolve()
    workdir = Path(args.workdir).expanduser().resolve()
    session_id = args.session_id or (
        f"grouped-{re.sub(r'[^A-Za-z0-9_-]', '-', args.expected_version)}-{uuid.uuid4().hex[:12]}"
    )
    result: dict[str, Any] = {
        "schema": SCHEMA,
        "status": "running",
        "created_at": iso_from_ns(clock.origin_wall_ns),
        "trial": args.trial,
        "session_id": session_id,
        "expected_version": args.expected_version,
        "environment": {
            "python": sys.version,
            "python_executable": sys.executable,
            "platform": sys.platform,
            "pid": os.getpid(),
            "cwd": os.getcwd(),
            "omni_home": str(omni_home),
            "workdir": str(workdir),
            "home": os.environ.get("HOME", ""),
            "pythonpath": os.environ.get("PYTHONPATH", ""),
            "fresh_home_required": not args.reuse_home,
            "preflight": args.preflight,
        },
        "plan": plan,
        "setup": {},
        "startup": {},
        "turns": [],
        "events": [],
        "cleanup": {"attempted": False, "clean": False},
    }

    ensure_fresh_home(omni_home, args.reuse_home)
    workdir.mkdir(parents=True, exist_ok=True)
    os.environ["OMNI_HOME"] = str(omni_home)
    # A missing/incorrect cache must fail honestly instead of silently changing
    # models during a trial or spending five seconds on the public registry.
    os.environ["OMNI_REGISTRY"] = "http://127.0.0.1:1/grouped-live-offline"
    dial_path = write_dial(omni_home, plan["models"])
    result["setup"]["dial_cache"] = {
        "path": str(dial_path),
        "content": json.loads(dial_path.read_text(encoding="utf-8")),
    }

    processes = ProcessTracker(clock, args.process_poll_ms)
    processes.start()
    processes.mark("harness:before_import")
    chat = None
    architecture = ""
    daemon_may_have_started = False
    cli_paths: dict[str, str | None] = {}
    recorder = Recorder(clock)
    failure: BaseException | None = None

    try:
        import_begin = time.perf_counter_ns()
        _omni, Inference, module_info = import_and_assert_omni(args)
        import_end = time.perf_counter_ns()
        architecture = module_info["architecture"]
        result["module"] = module_info
        result["setup"]["import_ms"] = ms_between(import_begin, import_end)

        cli_paths = {name: shutil.which(name) for name in ("claude", "codex", "agy")}
        result["setup"]["cli_paths"] = cli_paths
        if not args.skip_cli_check:
            missing = [name for name, path in cli_paths.items() if not path]
            if missing:
                raise RuntimeError(f"provider CLI(s) missing from PATH: {', '.join(missing)}")

        if args.preflight:
            preflight_begin = time.perf_counter_ns()
            if architecture == "daemon":
                daemon_may_have_started = True
            available = Inference.get_available_providers(["claude-code-cli", "codex-app-server", "antigravity-cli"])
            preflight_end = time.perf_counter_ns()
            result["setup"]["preflight"] = {
                "duration_ms": ms_between(preflight_begin, preflight_end),
                "available": list(available),
            }
            missing = sorted(set(PROVIDER_LABELS) - set(available))
            if missing:
                raise RuntimeError(f"preflight says providers unavailable: {', '.join(missing)}")
            if architecture == "daemon":
                result["daemon"] = assert_daemon(Inference, args, omni_home)

        # Positional provider argument works with both 0.3's ``providers_`` and
        # the daemon client's later ``providers`` spelling.
        chat = Inference.load_or_create_session(session_id, ["claude-code-cli"])
        chat.intelligence(0)
        chat.disable_subagents()
        chat.disable_mcp()
        chat.disable_autoremoving_unauthenticated_providers()
        chat.cwd(str(workdir))
        chat.logs(recorder.callback)

        startup_event_begin = len(recorder.events)
        start_begin_ns = time.perf_counter_ns()
        start_begin_wall_ns = time.time_ns()
        result["startup"]["start_begin_ms"] = clock.relative_ms(start_begin_ns)
        result["startup"]["start_begin_wall"] = iso_from_ns(start_begin_wall_ns)
        if architecture == "daemon":
            daemon_may_have_started = True
        chat.start()
        start_return_ns = time.perf_counter_ns()
        ready_ns = wait_idle(chat, args.turn_timeout, "chat.start")
        result["startup"].update(
            {
                "start_return_ms": clock.relative_ms(start_return_ns),
                "ready_ms": clock.relative_ms(ready_ns),
                "start_call_ms": ms_between(start_begin_ns, start_return_ns),
                "start_to_ready_ms": ms_between(start_begin_ns, ready_ns),
                "event_start_index": startup_event_begin,
                "event_end_index": len(recorder.events),
            }
        )
        processes.mark("chat:started_and_idle")

        active_provider = "claude-code-cli"
        for index, spec in enumerate(workload()):
            turn, active_provider, turn_failure = run_turn(
                chat,
                spec,
                recorder,
                processes,
                clock,
                args,
                plan["models"][spec.provider]["model"],
                active_provider,
            )
            result["turns"].append(turn)
            if turn_failure is not None:
                raise turn_failure
            # Assert the actual daemon only after the first model turn, so the
            # version check cannot pre-warm the timed first-use path.
            if index == 0 and architecture == "daemon" and "daemon" not in result:
                result["daemon"] = assert_daemon(Inference, args, omni_home)

        result["status"] = "completed"
    except BaseException as exc:
        failure = exc
        result["status"] = "failed"
        result["failure"] = {
            "type": type(exc).__name__,
            "message": str(exc),
            "traceback": traceback.format_exc(),
        }
    finally:
        result["cleanup"]["attempted"] = True
        cleanup_begin = time.perf_counter_ns()
        cleanup_errors: list[str] = []
        if chat is not None:
            try:
                chat.stop()
            except Exception as exc:
                cleanup_errors.append(f"chat.stop: {exc!r}")
        result["cleanup"]["chat_stop_ms"] = ms_between(
            cleanup_begin, time.perf_counter_ns()
        )

        daemon_stop_result: bool | None = None
        if architecture == "daemon" and daemon_may_have_started:
            daemon_stop_begin = time.perf_counter_ns()
            try:
                daemon_module = importlib.import_module("omni.client.daemon")
                paths_module = importlib.import_module("omni.shared.paths")
                # ``daemon.stop()`` opens a Link, and Link startup is automatic.
                # Guard it so cleanup cannot start a daemon after a failed start.
                if daemon_module.listening(paths_module.socket()):
                    daemon_stop_result = bool(daemon_module.stop())
                    if not daemon_stop_result:
                        cleanup_errors.append("daemon.stop returned false")
            except Exception as exc:
                cleanup_errors.append(f"daemon.stop: {exc!r}")
            result["cleanup"]["daemon_stop_ms"] = ms_between(
                daemon_stop_begin, time.perf_counter_ns()
            )
        result["cleanup"]["daemon_stop_result"] = daemon_stop_result

        deadline = time.monotonic() + args.cleanup_timeout
        alive = processes.alive_known()
        while alive and time.monotonic() < deadline:
            time.sleep(0.05)
            alive = processes.alive_known()
        result["cleanup"]["leaked_known_processes"] = alive
        result["cleanup"]["errors"] = cleanup_errors
        result["cleanup"]["clean"] = not alive and not cleanup_errors
        result["cleanup"]["total_ms"] = ms_between(
            cleanup_begin, time.perf_counter_ns()
        )
        processes.mark("harness:after_cleanup")
        processes.stop()
        # Do this only after timing and process-leak accounting: even a harmless
        # ``--version`` invocation can warm executable/runtime filesystem pages.
        result["setup"]["cli_versions_after_workload"] = probe_cli_versions(cli_paths)
        result["events"] = recorder.events
        result["callback_failures"] = recorder.callback_failures
        result["process_tracking"] = processes.report()
        result["summary"] = summarize(result)
        result["finished_at"] = iso_from_ns(time.time_ns())
        result["elapsed_ms_including_cleanup_and_postflight"] = round(
            (time.perf_counter_ns() - clock.origin_perf_ns) / 1_000_000, 3
        )
        if failure is None and not result["summary"]["overall"]["pass"]:
            result["status"] = "invalid"
    return result


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.turn_timeout <= 0 or args.idle_timeout <= 0 or args.cleanup_timeout <= 0:
        raise SystemExit("timeouts must be positive")
    if args.process_poll_ms <= 0 or args.settle_ms < 0:
        raise SystemExit("process polling must be positive and settle-ms nonnegative")
    if args.dry_run:
        dry = {
            "schema": SCHEMA,
            "dry_run": True,
            "expected_version": args.expected_version,
            "expect_architecture": args.expect_architecture,
            "omni_home": str(Path(args.omni_home).expanduser().resolve()),
            "workdir": str(Path(args.workdir).expanduser().resolve()),
            "plan": benchmark_plan(args),
            "note": "No omni import, process launch, provider call, or filesystem setup occurred.",
        }
        write_result(args.output, dry)
        return 0

    try:
        result = execute(args)
    except BaseException as exc:
        # Validation may fail before the structured runner owns cleanup. Keep
        # the CLI failure direct and avoid pretending a partial result is live.
        print(f"grouped_live: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2
    write_result(args.output, result)
    summary = result.get("summary", {}).get("overall", {})
    print(
        f"{result['status']}: {args.output}; "
        f"turns={summary.get('turn_count', 0)} pass={summary.get('pass', False)}",
        file=sys.stderr,
    )
    return 0 if result["status"] == "completed" else 2


if __name__ == "__main__":
    raise SystemExit(main())
