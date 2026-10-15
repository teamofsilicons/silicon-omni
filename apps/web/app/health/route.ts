import { NextResponse } from "next/server"
import { readCatalog } from "../../lib/store"
export const dynamic = "force-dynamic"
export async function GET() {
  try {
    const { catalog, revision, writable } = await readCatalog()
    return NextResponse.json({ status: "ok", service: "omni", models: catalog.models.length,
      categories: Object.keys(catalog.keys).length, storage: writable ? "s3" : "bundled", revision },
      { headers: { "cache-control": "no-store" } })
  } catch {
    return NextResponse.json({ status: "unavailable", service: "omni" }, { status: 503 })
  }
}
