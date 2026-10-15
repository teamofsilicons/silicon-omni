import { NextResponse } from "next/server"
import { readCatalog } from "../../lib/store"
export const dynamic = "force-dynamic"
export async function GET() {
  try {
    const { catalog, revision } = await readCatalog()
    return NextResponse.json({ categories: Object.entries(catalog.keys).map(([id, k]) => ({ id, ...k })), revision },
      { headers: { "cache-control": "public, max-age=0, s-maxage=30" } })
  } catch {
    return NextResponse.json({ error: "Catalog is temporarily unavailable." }, { status: 503 })
  }
}
