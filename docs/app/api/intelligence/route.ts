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

import seed from "../../../data/seed.json"
import { allDials, dialFor } from "../../../lib/dial"
import { configured, entries } from "../../../lib/db"
import { combinations, key } from "../../../lib/providers"

export const dynamic = "force-dynamic"

export async function GET(request: NextRequest) {
  const asked = request.nextUrl.searchParams.get("providers") ?? ""
  const providers = asked.split(/[+,\s]+/).filter(Boolean)
  const rows = await entries()
  const combos = combinations()

  return NextResponse.json(
    {
      providers: providers.length ? key(providers) : null,
      ladder: providers.length ? dialFor(rows, providers) : null,
      ladders: allDials(rows, combos),
      source: seed.source,
      caveats: seed.caveats,
      live: configured,
      counted: rows.length,
    },
    { headers: { "cache-control": "public, s-maxage=60, stale-while-revalidate=3600" } },
  )
}
