/**
 * The answer. This is the URL the library reads.
 *
 *   /choose.json?key=code&providers=claude-code-cli+codex-app-server
 *   /choose.json?intelligence=7&providers=claude-code-cli+codex-app-server&bench=gdpval-aa
 *   /choose.json?model=gemini-3.8-flash-low&effort=&fast=false
 *
 * Three ways to ask, because no one of them is enough on its own. A key is a
 * shortlist somebody chose. A number is the dial. A model is you already
 * knowing, and it is passed to the CLI verbatim — a model released this
 * morning works without this file knowing about it.
 *
 * Whatever the question, the answer is a command that runs.
 */

import { NextRequest, NextResponse } from "next/server"

import { byCuratedIntelligence, byIntelligence, byKey, byName, unusable, type AvailableModels, type Key } from "../../lib/choose"
import { graph, VIEW } from "../../lib/graph"
import { canRunFast, invocation, PROVIDERS } from "../../lib/providers"

export const dynamic = "force-dynamic"

const ALL: string[] = PROVIDERS.map((p) => p.id)

/** "false", "0" and "" are all false. Anything else present is true. */
function flag(value: unknown): boolean {
  if (value === true) return true
  if (value === null || value === undefined) return false
  const s = String(value).trim().toLowerCase()
  return s !== "" && s !== "false" && s !== "0" && s !== "no"
}

function shown(pick: { provider: string; model: string; effort: string; fast: boolean } | null) {
  if (!pick) return null
  return {
    provider: pick.provider,
    model: pick.model,
    effort: pick.effort,
    fast: pick.fast,
    run: invocation(pick),
  }
}

async function answer(asked: Record<string, unknown>) {
  let availableModels: AvailableModels = {}
  if (asked.available_models !== undefined) {
    try {
      const raw = typeof asked.available_models === "string"
        ? JSON.parse(asked.available_models) : asked.available_models
      if (!raw || typeof raw !== "object" || Array.isArray(raw) ||
          Object.entries(raw).some(([id, models]) => !ALL.includes(id) || !Array.isArray(models) || models.length > 1000 ||
            models.some(m => typeof m !== "string" || !/^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,119}$/.test(m)))) throw new Error()
      availableModels = raw
    } catch {
      return NextResponse.json({ error: "available_models must map provider IDs to arrays of model IDs" }, { status: 400 })
    }
  }
  const { models, benches, keys, source, caveats, fresh } = await graph()

  const wanted = Array.isArray(asked.providers)
    ? asked.providers.map(String)
    : String(asked.providers ?? "").split(/[+,\s]+/).filter(Boolean)
  const providers = (wanted.length ? wanted : ALL).filter((p) => ALL.includes(p))
  if (!providers.length) {
    return NextResponse.json({ error: "no such provider", asked: wanted, available: ALL }, { status: 400 })
  }

  const head = { headers: { "cache-control": "public, s-maxage=60, stale-while-revalidate=3600" } }
  const foot = { providers: [...providers].sort(), source: { ...source, database: VIEW }, fresh }

  /* You already know. Checked, not second-guessed. */
  if (asked.model) {
    const named = byName(providers, {
      provider: asked.provider ? String(asked.provider) : undefined,
      model: String(asked.model),
      effort: asked.effort === undefined || asked.effort === null ? "" : String(asked.effort),
      fast: flag(asked.fast),
    })
    if (!named.pick) return NextResponse.json({ error: named.why, ...foot }, { status: 400 })
    return NextResponse.json({ how: named.how, pick: shown(named.pick), ...foot }, head)
  }

  /* A shortlist somebody chose. */
  if (asked.key) {
    const key = String(asked.key).trim().toLowerCase()
    const found = byKey(keys as Record<string, Key>, key, providers, availableModels)
    if (!found) {
      return NextResponse.json(
        {
          error: "no such key",
          asked: key,
          available: Object.entries(keys).map(([id, k]) => ({ id, says: (k as Key).says })),
        },
        { status: 400 },
      )
    }
    if (!found.pick) {
      return NextResponse.json(
        { error: `nothing on the "${key}" shortlist is signed in`, ...found, shortlist: undefined, ...foot },
        { status: 409 },
      )
    }
    return NextResponse.json(
      {
        how: found.how,
        key: found.key,
        says: found.says,
        pick: shown(found.pick),
        shortlist: found.shortlist.map((p) => ({ ...shown(p)!, available: p.available,
          ...(p.requires_availability ? { requires_availability: true } : {}),
        })),
        ...foot,
      },
      head,
    )
  }

  /* The default dial is curated; a named benchmark asks for measured results. */
  const n = Number(asked.intelligence ?? 5)
  if (!Number.isFinite(n) || n < 0 || n > 10) {
    return NextResponse.json(
      { error: "intelligence must be a number from 0 to 10", asked: asked.intelligence },
      { status: 400 },
    )
  }
  const askedBench = asked.bench ?? asked.benchmark
  if (askedBench === undefined || askedBench === null) {
    const dial = byCuratedIntelligence(keys, providers, n, availableModels)
    return NextResponse.json(
      {
        ...dial,
        pick: shown(dial.pick),
        ladder: Object.fromEntries(Object.entries(dial.ladder).map(([l, p]) => [l, shown(p)])),
        ...(dial.pick ? {} : { reason: `no eligible model on the "${dial.category}" shortlist` }),
        caveats,
        ...foot,
      },
      head,
    )
  }
  const bench = resolveBench(String(askedBench), Object.keys(benches), benches)
  if (!bench) {
    return NextResponse.json(
      {
        error: "no such benchmark",
        asked: asked.bench ?? asked.benchmark,
        available: Object.entries(benches).map(([id, b]) => ({ id, label: b.label })),
      },
      { status: 400 },
    )
  }
  const dial = byIntelligence(models, providers, bench, n, availableModels)
  return NextResponse.json(
    {
      how: dial.how,
      basis: dial.basis,
      intelligence: dial.intelligence,
      bench,
      benchmark: benches[bench],
      pick: shown(dial.pick),
      ...(dial.pick ? {} : { reason: "no current model has the required benchmark, cost, and time measurements" }),
      ladder: Object.fromEntries(Object.entries(dial.ladder).map(([l, p]) => [l, shown(p)])),
      skipped: unusable(models, bench)
        .filter((u) => providers.includes(u.entry.provider))
        .map(({ entry, why }) => ({ provider: entry.provider, model: entry.model, effort: entry.effort ?? "", why })),
      caveats,
      ...foot,
    },
    head,
  )
}

/** lowercase, hyphens between words, nothing else. */
function slug(text: string): string {
  return text.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "")
}

/** the same, with any trailing version dropped: "terminal-bench-2-1" -> "terminal-bench" */
function versionless(text: string): string {
  let out = slug(text)
  while (/-(v?\d+)$/.test(out)) out = out.replace(/-(v?\d+)$/, "")
  return out
}

/**
 * Find the board somebody means: the id, the label with its version dropped,
 * the label whole, then an unambiguous prefix — so `design-arena` reaches
 * `design-arena-full-stack` while anything matching two boards is refused.
 */
function resolveBench(asked: string, ids: string[], benches: Record<string, { label: string }>): string | null {
  const want = versionless(asked)
  const exact = ids.find((id) => versionless(id) === want)
  if (exact) return exact
  const byLabel = ids.find((id) => versionless(benches[id].label) === want)
  if (byLabel) return byLabel
  const whole = ids.find((id) => slug(benches[id].label) === slug(asked))
  if (whole) return whole
  const starts = ids.filter((id) => versionless(id).startsWith(want) && want.length > 2)
  return starts.length === 1 ? starts[0] : null
}

export async function GET(request: NextRequest) {
  const q = request.nextUrl.searchParams
  return answer(Object.fromEntries(q.entries()))
}

export async function POST(request: NextRequest) {
  try {
    return answer(await request.json())
  } catch {
    return NextResponse.json({ error: "body must be json" }, { status: 400 })
  }
}
