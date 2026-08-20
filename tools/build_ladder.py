"""Build ``omni/intelligence/ladder.json`` from GDPval-AA v2.

omni itself does not do any of this. The dial is a finished map from level to
model, and this is what produces it — the same job the remote registry will do
once it exists. Until then the packaged file is not a fallback, it is the only
source, so the ``captured`` date it carries is worth looking at. Re-run this
whenever the leaderboard moves::

    python3 tools/build_ladder.py

Every model our three CLIs can run is plotted by its GDPval-AA v2 Elo against
the dollars Artificial Analysis measured it cost to earn that score. Only the
left edge is kept: a model survives if nothing else is both better and cheaper.
Level 10 is the top of the edge, and the walk goes down and to the left.

The edge is computed once per set of providers, because losing a provider puts
models back on the dial that another vendor's were shadowing.
"""

import datetime
import itertools
import json
import re
import urllib.request
from pathlib import Path

LEADERBOARD = "https://artificialanalysis.ai/evaluations/gdpval-aa"
ANY_MODEL = "https://artificialanalysis.ai/models/claude-sonnet-5"  # any model page carries all 204
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "omni" / "intelligence" / "ladder.json"
GRAPH = ROOT / "docs" / "data" / "models.json"  # the same graph, for the site to serve
LEVELS = 11
PROVIDERS = ("claude", "openai", "google")

#: Artificial Analysis reports GDPval cost weighted for its Intelligence Index.
#: Times five is the published dollars-per-task; verified against all 18 rows
#: the leaderboard labels, exact to five decimals.
WEIGHTED_TO_TASK = 5

#: our CLI slug and effort -> the leaderboard row it is. Hand-checked; a model
#: with no exact row is left out rather than matched to something near it.
MODELS = [
    ("claude", "claude-opus-5", "max", "Claude Opus 5 (Adaptive Reasoning, Max Effort)"),
    ("claude", "claude-opus-5", "xhigh", "Claude Opus 5 (Adaptive Reasoning, Xhigh Effort)"),
    ("claude", "claude-opus-5", "high", "Claude Opus 5 (Adaptive Reasoning, High Effort)"),
    ("claude", "claude-opus-5", "medium", "Claude Opus 5 (Adaptive Reasoning, Medium Effort)"),
    ("claude", "claude-opus-5", "low", "Claude Opus 5 (Adaptive Reasoning, Low Effort)"),
    ("claude", "claude-sonnet-5", "max", "Claude Sonnet 5 (Adaptive Reasoning, Max Effort)"),
    ("claude", "claude-haiku-4-5-20251001", "", "Claude 4.5 Haiku (Reasoning)"),
    ("openai", "gpt-5.6-sol", "max", "GPT-5.6 Sol (max)"),
    ("openai", "gpt-5.6-sol", "xhigh", "GPT-5.6 Sol (xhigh)"),
    ("openai", "gpt-5.6-sol", "high", "GPT-5.6 Sol (high)"),
    ("openai", "gpt-5.6-sol", "medium", "GPT-5.6 Sol (medium)"),
    ("openai", "gpt-5.6-sol", "low", "GPT-5.6 Sol (low)"),
    ("openai", "gpt-5.6-terra", "max", "GPT-5.6 Terra (max)"),
    ("openai", "gpt-5.6-terra", "xhigh", "GPT-5.6 Terra (xhigh)"),
    ("openai", "gpt-5.6-terra", "high", "GPT-5.6 Terra (high)"),
    ("openai", "gpt-5.6-terra", "medium", "GPT-5.6 Terra (medium)"),
    ("openai", "gpt-5.6-terra", "low", "GPT-5.6 Terra (low)"),
    ("openai", "gpt-5.6-luna", "max", "GPT-5.6 Luna (max)"),
    ("openai", "gpt-5.6-luna", "xhigh", "GPT-5.6 Luna (xhigh)"),
    ("openai", "gpt-5.6-luna", "high", "GPT-5.6 Luna (high)"),
    ("openai", "gpt-5.6-luna", "medium", "GPT-5.6 Luna (medium)"),
    ("openai", "gpt-5.6-luna", "low", "GPT-5.6 Luna (low)"),
    ("openai", "gpt-5.5", "xhigh", "GPT-5.5 (xhigh)"),
    ("openai", "gpt-5.5", "high", "GPT-5.5 (high)"),
    ("openai", "gpt-5.5", "medium", "GPT-5.5 (medium)"),
    ("openai", "gpt-5.5", "low", "GPT-5.5 (low)"),
    ("openai", "gpt-5.4", "xhigh", "GPT-5.4 (xhigh)"),
    ("openai", "gpt-5.4-mini", "xhigh", "GPT-5.4 mini (xhigh)"),
    ("google", "gemini-3.7-flash-high", "", "Gemini 3.7 Flash (high)"),
    ("google", "gemini-3.7-flash-medium", "", "Gemini 3.7 Flash (medium)"),
    ("google", "gemini-3.7-flash-low", "", "Gemini 3.7 Flash (low)"),
    ("google", "gemini-3.6-flash-high", "", "Gemini 3.6 Flash (high)"),
    ("google", "gemini-3.5-flash-high", "", "Gemini 3.5 Flash (high)"),
]

CAVEATS = [
    "Left out because Artificial Analysis has no GDPval-AA v2 score for them: "
    "gpt-5.3-codex-spark, gemini-3.1-pro-high/low (only a Preview build is scored), "
    "claude-opus-4-6-thinking, gpt-oss-120b-medium (only high and low are scored), "
    "gemini-3.6-flash and gemini-3.5-flash below high.",
    "Left out because AA scores them but publishes no cost: claude-sonnet-5 below max effort.",
    "claude-fable-5 is left out: AA's only Fable 5 row is a router that can serve Claude Opus 4.8, "
    "so its Elo does not describe what `claude --model claude-fable-5` runs.",
    "claude-sonnet-4-6 through agy is left out: AA measured it at max effort and agy exposes no "
    "effort control, so the score may not describe what the CLI runs.",
    "claude-haiku-4-5-20251001 carries no effort because AA scored one 'Reasoning' configuration "
    "with no effort split.",
    "Gemini prices are promotional through 2026-12-31 and double on 2027-01-01, which will move "
    "those points right. AA also measured Gemini through Google's own API, not through agy.",
    "AA bills OpenAI cache writes at 1.25x input, which OpenAI's own rate card does not charge, so "
    "GPT-5.6 costs run about 3% high. It is applied consistently, so the ordering is unaffected.",
]


def page(url: str) -> str:
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
    """Dollars per GDPval task, as Artificial Analysis measured it."""
    node = record.get("intelligenceIndexCostPerTask") or {}
    for entry in node.get("evaluations") or []:
        if entry.get("slug") == "gdpval-aa":
            return round(entry["weightedCostPerTask"] * WEIGHTED_TO_TASK, 4)
    return None


def beats(one: dict, other: dict) -> bool:
    return (
        one["score"] >= other["score"]
        and one["price"] <= other["price"]
        and (one["score"] > other["score"] or one["price"] < other["price"])
    )


def edge(points: list[dict]) -> list[dict]:
    """The left edge of the graph, best first: nothing here is beaten on both axes."""
    kept = [p for p in points if not any(beats(q, p) for q in points)]
    out, seen = [], set()
    for point in sorted(kept, key=lambda p: (-p["score"], p["price"])):
        spot = (point["score"], point["price"])
        if spot not in seen:  # two models on one spot are one point
            seen.add(spot)
            out.append(point)
    return out


def dial(points: list[dict]) -> dict:
    """Spread the edge over levels 0-10, 10 at the top."""
    if not points:
        return {}
    steps = len(points) - 1
    return {
        str(level): points[round((LEVELS - 1 - level) * steps / (LEVELS - 1))]
        for level in range(LEVELS)
    }


def graph() -> list[dict]:
    rows = {r["name"]: r for r in objects(page(ANY_MODEL), r'"gdpval":\s*-?\d') if "gdpval" in r}
    points, dropped = [], []
    for provider, model, effort, name in MODELS:
        row = rows.get(name)
        price = cost(row) if row else None
        if price is None:
            dropped.append(name)
            continue
        points.append(
            {
                "provider": provider,
                "model": model,
                "effort": effort,
                "score": round(row["gdpval"], 1),
                "price": price,
            }
        )
    if dropped:
        print(f"  no score or cost, skipped: {', '.join(dropped)}")
    return points


def build(points: list[dict]) -> dict:
    ladders = {}
    for size in range(1, len(PROVIDERS) + 1):
        for combo in itertools.combinations(PROVIDERS, size):
            key = "+".join(sorted(combo))
            mine = [p for p in points if p["provider"] in combo]
            rungs = edge(mine)
            ladders[key] = dial(rungs)
            print(f"  {key:26} {len(mine):2} plotted -> {len(rungs)} rungs")
    return {
        "version": 1,
        "source": {
            "benchmark": "GDPval-AA v2, Artificial Analysis",
            "url": LEADERBOARD,
            "score": "Elo from blind pairwise judging of complete work deliverables over 220 GDPval "
            "tasks in an agentic harness, anchored so a human expert scores 1000. 95% CIs run about "
            "+/-15 to +/-27, so gaps under ~35 Elo are not separable.",
            "price": "USD per GDPval task, as Artificial Analysis measured it on the same runs.",
            "built_by": "tools/build_ladder.py",
            "captured": datetime.date.today().isoformat(),
        },
        "note": "One finished dial per set of providers: level 10 down to level 0, already reduced "
        "to the leftmost models on the score-versus-price graph. Each level is walked down and to "
        "the left of the one above, so a step down is always cheaper and never a sideways move. "
        "'model' and 'effort' go to the CLI verbatim; an empty effort means the flag is not passed. "
        "There is one dial per provider set because losing a provider puts models back on the dial "
        "that another vendor's were shadowing.",
        "caveats": CAVEATS,
        "ladders": ladders,
    }


if __name__ == "__main__":
    print(f"reading {ANY_MODEL}")
    plotted = graph()
    print(f"  {len(plotted)} models plotted")
    doc = build(plotted)
    OUT.write_text(json.dumps(doc, indent=2) + "\n")
    print(f"wrote {OUT}")
    # The site serves the whole graph, not the dials, so it can recompute the
    # edge itself as models are added and removed by hand.
    existing = json.loads(GRAPH.read_text()) if GRAPH.exists() else {}
    GRAPH.parent.mkdir(parents=True, exist_ok=True)
    GRAPH.write_text(
        json.dumps(
            {
                "note": existing.get("note", ""),
                "source": doc["source"],
                "caveats": doc["caveats"],
                "models": [dict(p, note=f"GDPval-AA v2, {doc['source']['captured']}") for p in plotted],
            },
            indent=2,
            ensure_ascii=False,
        )
        + "\n"
    )
    print(f"wrote {GRAPH} ({len(plotted)} models)")
