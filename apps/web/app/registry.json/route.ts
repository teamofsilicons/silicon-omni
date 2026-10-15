/**
 * What you may ask for.
 *
 *   GET /registry.json
 *
 * Every benchmark you can be ranked on and every provider that can answer,
 * so a client never has to guess at a spelling or ship a list that goes stale.
 */

import { NextResponse } from "next/server"

import type { Key } from "../../lib/choose"
import { graph, VIEW } from "../../lib/graph"
import { canRunFast, PROVIDERS } from "../../lib/providers"

export const dynamic = "force-dynamic"

export async function GET() {
  const { models, benches, keys, source, fresh } = await graph()

  return NextResponse.json(
    {
      benches: Object.entries(benches).map(([id, b]) => ({
        id,
        label: b.label,
        url: b.url,
        // how many of the models on the board carry a score for it
        scored: models.filter((m) => m.scores?.[id] !== undefined && m.scores?.[id] !== null).length,
      })),
      providers: PROVIDERS.map((p) => ({
        id: p.id,
        label: p.label,
        cli: p.cli,
        efforts: p.efforts,
        /* whether asking this one to run hot means anything */
        fast: canRunFast(p.id),
        models: models.filter((m) => m.provider === p.id).length,
      })),
      /* Three ways to ask, and what each of them takes. */
      how: [
        { id: "key", takes: "one of `keys` below", means: "a shortlist somebody chose. the first vendor on it you are signed into answers" },
        { id: "intelligence", takes: "0-10, and an optional `bench`", means: "the left edge of that board, spread over eleven rungs" },
        { id: "model", takes: "`model`, optional `effort` and `fast`", means: "you already know. passed to the CLI verbatim" },
      ],
      keys: Object.entries(keys as Record<string, Key>).map(([id, k]) => ({
        id,
        says: k.says,
        picks: k.picks.map((m) => ({
          provider: m.provider,
          model: m.model,
          effort: m.effort ?? "",
          fast: Boolean(m.fast),
        })),
      })),
      models: models.length,
      source: { ...source, database: VIEW },
      fresh,
    },
    { headers: { "cache-control": "public, s-maxage=60, stale-while-revalidate=3600" } },
  )
}
