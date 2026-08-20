/**
 * The dial, as omni asks for it.
 *
 *   GET /api/intelligence?providers=claude+google
 *
 * Answers with the 0-10 map for exactly those providers, plus every other
 * combination alongside it so a client can cache more than it asked for. omni
 * reads `ladders[<providers>]` and falls back to `ladder`.
 */

import { NextRequest, NextResponse } from "next/server"

import { allDials, dialFor } from "../../../lib/dial"
import { graph, SOURCE, VIEW } from "../../../lib/graph"
import { combinations, key } from "../../../lib/providers"

export const dynamic = "force-dynamic"

export async function GET(request: NextRequest) {
  const asked = request.nextUrl.searchParams.get("providers") ?? ""
  const providers = asked.split(/[+,\s]+/).filter(Boolean)
  const { models, source, caveats, fresh } = await graph()

  return NextResponse.json(
    {
      providers: providers.length ? key(providers) : null,
      ladder: providers.length ? dialFor(models, providers) : null,
      ladders: allDials(models, combinations()),
      source: { ...source, models: SOURCE, edit: VIEW },
      caveats,
      counted: models.length,
      fresh,
    },
    { headers: { "cache-control": "public, s-maxage=60, stale-while-revalidate=3600" } },
  )
}
