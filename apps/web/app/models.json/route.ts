import { NextRequest, NextResponse } from "next/server"
import { readCatalog } from "../../lib/store"
import { PROVIDERS } from "../../lib/providers"
export const dynamic = "force-dynamic"
export async function GET(request: NextRequest) {
  const provider = request.nextUrl.searchParams.get("provider")
  if (provider && !PROVIDERS.some(p => p.id === provider))
    return NextResponse.json({ error: "Unknown provider." }, { status: 400 })
  try {
    const { catalog, revision } = await readCatalog()
    const models = catalog.models.filter(m => !provider || m.provider === provider)
    return NextResponse.json({ models, revision, unique_models: new Set(models.map(m => m.model)).size },
      { headers: { "cache-control": "public, max-age=0, s-maxage=30", "Access-Control-Allow-Origin": "*" } })
  } catch {
    return NextResponse.json({ error: "Catalog is temporarily unavailable." }, { status: 503 })
  }
}
