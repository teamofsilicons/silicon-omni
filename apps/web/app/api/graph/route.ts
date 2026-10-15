/** Every model on the graph, whether or not it currently holds a level. */

import { NextResponse } from "next/server"

import { EDIT, graph, SOURCE, VIEW } from "../../../lib/graph"

export const dynamic = "force-dynamic"

export async function GET() {
  const { models, source, caveats, fresh } = await graph()
  return NextResponse.json({
    models,
    source,
    caveats,
    fresh,
    links: { raw: SOURCE, edit: EDIT, view: VIEW },
  })
}
