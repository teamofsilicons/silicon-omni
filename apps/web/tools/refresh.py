"""Refresh models.json from Artificial Analysis.

The file is meant to be hand-edited — add a model the day it ships. This is for
the other case: pulling the whole board again when the leaderboards move.

    python3 tools/refresh.py

This only updates the local seed; publish reviewed measurements through the admin API.
It never writes to production storage.

It collects, for every model: one score per benchmark, the dollars a task costs
and the seconds a task takes. Working out which model to run is the site's job,
and is done in one place: lib/choose.ts.
"""

import datetime
import json
import re
import statistics
import urllib.request
from pathlib import Path

# Any model page carries the whole comparison set in its payload.
SOURCE = "https://artificialanalysis.ai/models/gpt-6-astra"
OUT = Path(__file__).resolve().parent.parent / "models.json"

#: The benchmarks Artificial Analysis publishes for us, and the field each one
#: is in their payload. The name carries no version and the label does, so
#: Terminal-Bench 4.0 can become 3 without every caller having to change.
#: design-arena-full-stack and agents-last-exam come from elsewhere and are
#: hand-recorded in models.json; this tool leaves them alone.
#: The benchmarks a caller may choose between, and the field each one is in the
#: Artificial Analysis payload. A benchmark only belongs here if every model we
#: list has a score for it — a bench that is null for half the board would rank
#: half the board last, which is worse than not offering it.
BENCHES = {
    "artificial-analysis-intelligence-index": ("intelligenceIndex", "Artificial Analysis Intelligence Index v4.3",
                 "https://artificialanalysis.ai/methodology/intelligence-benchmarking"),
    "gdpval-aa": ("gdpval", "GDPval-AA v2",
                 "https://artificialanalysis.ai/evaluations/gdpval-aa"),
    "terminal-bench": ("terminalbenchV40", "Terminal-Bench 4.0",
                 "https://artificialanalysis.ai/evaluations/terminal-bench"),
}

#: our CLI slug and effort -> the leaderboard row it is. Hand-checked. A model
#: with no exact row is left out rather than matched to something near it.
MODELS = [
    ("codex-app-server", "gpt-6-astra", "max", "GPT-6 Astra (max)"),
    ("codex-app-server", "gpt-6-astra", "xhigh", "GPT-6 Astra (xhigh)"),
    ("codex-app-server", "gpt-6-astra", "high", "GPT-6 Astra (high)"),
    ("codex-app-server", "gpt-6-astra", "medium", "GPT-6 Astra (medium)"),
    ("codex-app-server", "gpt-6-astra", "low", "GPT-6 Astra (low)"),
    ("claude-code-cli", "claude-opus-5", "max", "Claude Opus 5 (Adaptive Reasoning, Max Effort)"),
    ("claude-code-cli", "claude-opus-5", "xhigh", "Claude Opus 5 (Adaptive Reasoning, Xhigh Effort)"),
    ("claude-code-cli", "claude-opus-5", "high", "Claude Opus 5 (Adaptive Reasoning, High Effort)"),
    ("claude-code-cli", "claude-opus-5", "medium", "Claude Opus 5 (Adaptive Reasoning, Medium Effort)"),
    ("claude-code-cli", "claude-opus-5", "low", "Claude Opus 5 (Adaptive Reasoning, Low Effort)"),
    ("claude-code-cli", "claude-sonnet-5", "max", "Claude Sonnet 5 (Adaptive Reasoning, Max Effort)"),
    ("claude-code-cli", "claude-haiku-4-5-20251001", "", "Claude 4.5 Haiku (Reasoning)"),
    ("codex-app-server", "gpt-5.6-terra", "max", "GPT-5.6 Terra (max)"),
    ("codex-app-server", "gpt-5.6-terra", "xhigh", "GPT-5.6 Terra (xhigh)"),
    ("codex-app-server", "gpt-5.6-terra", "high", "GPT-5.6 Terra (high)"),
    ("codex-app-server", "gpt-5.6-terra", "medium", "GPT-5.6 Terra (medium)"),
    ("codex-app-server", "gpt-5.6-terra", "low", "GPT-5.6 Terra (low)"),
    ("codex-app-server", "gpt-5.5", "xhigh", "GPT-5.5 (xhigh)"),
    ("codex-app-server", "gpt-5.5", "high", "GPT-5.5 (high)"),
    ("codex-app-server", "gpt-5.5", "medium", "GPT-5.5 (medium)"),
    ("codex-app-server", "gpt-5.5", "low", "GPT-5.5 (low)"),
    ("codex-app-server", "gpt-5.4", "xhigh", "GPT-5.4 (xhigh)"),
    ("codex-app-server", "gpt-5.4-mini", "xhigh", "GPT-5.4 mini (xhigh)"),
    ("antigravity-cli", "gemini-3.8-flash-high", "", "Gemini 3.8 Flash (high)"),
    ("antigravity-cli", "gemini-3.8-flash-medium", "", "Gemini 3.8 Flash (medium)"),
    ("antigravity-cli", "gemini-3.8-flash-low", "", "Gemini 3.8 Flash (low)"),
    ("antigravity-cli", "gemini-3.6-flash-high", "", "Gemini 3.6 Flash (high)"),
    ("antigravity-cli", "gemini-3.5-flash-high", "", "Gemini 3.5 Flash (high)"),
]

def payload(url: str) -> str:
    """A page's Next.js flight payload, decoded."""
    request = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(request, timeout=90) as response:
        html = response.read().decode("utf-8", "replace")
    chunks = re.findall(r'self\.__next_f\.push\(\[1,\s*("(?:[^"\\]|\\.)*")\]\)', html)
    return "".join(json.loads(chunk) for chunk in chunks)


def objects(blob: str, marker: str):
    """Every balanced JSON object in ``blob`` that contains ``marker``."""
    for found in re.finditer(marker, blob):
        depth, start = 0, None
        for i in range(found.start(), -1, -1):
            if blob[i] == "}":
                depth += 1
            elif blob[i] == "{":
                if depth == 0:
                    start = i
                    break
                depth -= 1
        if start is None:
            continue
        depth = 0
        for j in range(start, len(blob)):
            if blob[j] == "{":
                depth += 1
            elif blob[j] == "}":
                depth -= 1
                if depth == 0:
                    try:
                        yield json.loads(blob[start : j + 1])
                    except json.JSONDecodeError:
                        pass
                    break


def cost(record: dict):
    """Dollars a task costs, across the whole Intelligence Index.

    Deliberately not the GDPval-only figure the old file used: the caller now
    chooses which benchmark to be ranked on, so a cost measured on one of them
    would be the wrong number for the other three.
    """
    node = record.get("intelligenceIndexCostPerTask") or {}
    total = (node.get("cost") or {}).get("total")
    return round(total, 4) if total is not None else None


def seconds(record: dict):
    """Seconds a task takes, across the whole Intelligence Index."""
    value = record.get("intelligenceIndexTimePerTask")
    return round(value, 1) if value is not None else None


def name_key(provider, model, effort):
    return f"{provider}/{model}/{effort}"


if __name__ == "__main__":
    print(f"reading {SOURCE}")
    # hand-recorded boards survive a refresh of the ones we can pull
    prior = json.loads(OUT.read_text()) if OUT.exists() else {}
    existing = {name_key(m["provider"], m["model"], m.get("effort", "")): m.get("scores", {})
                for m in prior.get("models", []) if m["model"] != "gpt-6-astra" and not m["model"].startswith("gemini-3.8-flash")}
    rows = {r["name"]: r for r in objects(payload(SOURCE), r'"gdpval":\s*-?\d') if "gdpval" in r}
    models, missing = [], []
    for provider, model, effort, name in MODELS:
        row = rows.get(name)
        if row is None:
            missing.append((name, "no row"))
            continue
        scores = {}
        for bench, (field, _, _) in BENCHES.items():
            value = row.get(field)
            if value is not None:
                scores[bench] = round(value, 4)
        price, secs = cost(row), seconds(row)
        keep = {k: v for k, v in (existing.get(name_key(provider, model, effort)) or {}).items()
                if k not in BENCHES}
        scores.update(keep)
        models.append({"provider": provider, "model": model, "effort": effort,
                       "scores": scores, "price": price, "secs": secs})
    models.sort(key=lambda m: (-m["scores"].get("artificial-analysis-intelligence-index", 0), m["price"] if m["price"] is not None else float("inf")))
    for name, why in missing:
        print(f"  skipped {name}: {why}")

    doc = json.loads(OUT.read_text()) if OUT.exists() else {}
    # only the boards this tool pulls; anything hand-recorded stays put
    doc.setdefault("benches", {})
    for b, (field, label, url) in BENCHES.items():
        doc["benches"][b] = {**doc["benches"].get(b, {}), "label": label, "url": url, "field": field}
    doc["source"] = dict(doc.get("source", {}),
                         provider="Artificial Analysis",
                         captured=datetime.date.today().isoformat())
    doc["anchors"] = {
        "price mean": round(statistics.mean(m["price"] for m in models if m["price"] is not None), 4),
        "secs mean": round(statistics.mean(m["secs"] for m in models if m["secs"] is not None), 1),
    }
    # Keep manually added models, lifecycle labels, source links and API pricing.
    refreshed = {name_key(m["provider"], m["model"], m.get("effort", "")): m for m in models}
    merged = []
    for old in doc.get("models", []):
        identity = name_key(old["provider"], old["model"], old.get("effort", ""))
        update = refreshed.pop(identity, None)
        merged.append({**old, **update} if update else old)
    merged.extend(refreshed.values())
    doc["models"] = merged
    OUT.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {OUT} ({len(models)} models, {len(BENCHES)} benchmarks)")
