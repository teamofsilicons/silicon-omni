import { AuthError, authError, authJson, requireAdmin } from "@/lib/auth"
import { validateCatalog } from "@/lib/catalog"
import { CatalogConflict, readCatalog, writeCatalog } from "@/lib/store"

export const runtime = "nodejs"
export const dynamic = "force-dynamic"
const MAX_BODY_BYTES = 1024 * 1024

async function readBody(request: Request): Promise<Record<string, unknown>> {
  if (request.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "application/json")
    throw new AuthError(415, "invalid_request", "Send an application/json request.")
  if (Number(request.headers.get("content-length")) > MAX_BODY_BYTES)
    throw new AuthError(413, "invalid_request", "The catalog request must be at most 1 MiB.")
  const reader = request.body?.getReader()
  if (!reader) throw new AuthError(400, "invalid_request", "Send a catalog and its current revision.")
  const chunks: Uint8Array[] = []
  let length = 0
  try {
    for (;;) {
      const { value, done } = await reader.read()
      if (done) break
      length += value.byteLength
      if (length > MAX_BODY_BYTES) {
        await reader.cancel()
        throw new AuthError(413, "invalid_request", "The catalog request must be at most 1 MiB.")
      }
      chunks.push(value)
    }
  } finally { reader.releaseLock() }
  try {
    const body: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"))
    if (!body || typeof body !== "object" || Array.isArray(body)) throw new Error()
    return body as Record<string, unknown>
  } catch { throw new AuthError(400, "invalid_request", "Send a JSON object containing catalog and revision.") }
}

function failure(error: unknown): Response {
  if (error instanceof AuthError) return authError(error)
  if (error instanceof CatalogConflict) return authJson({ error: { code: "catalog_conflict", message: "Someone saved a newer catalog. Reload before publishing your changes." } }, 409)
  return authJson({ error: { code: "catalog_unavailable", message: "Catalog storage is unavailable. Try again shortly." } }, 503)
}

export async function GET(request: Request): Promise<Response> {
  try {
    await requireAdmin(request)
    return authJson(await readCatalog())
  } catch (error) { return failure(error) }
}

export async function PUT(request: Request): Promise<Response> {
  try {
    const session = await requireAdmin(request)
    const body = await readBody(request)
    if (typeof body.revision !== "string" || !/^"[a-f0-9-]+"$/.test(body.revision))
      throw new AuthError(400, "invalid_revision", "Reload the catalog to obtain its current revision.")
    let catalog
    try { catalog = validateCatalog(body.catalog) }
    catch (error) { throw new AuthError(400, "invalid_catalog", error instanceof Error ? error.message : "The catalog is invalid.") }
    return authJson(await writeCatalog(catalog, body.revision, session.account.uuid))
  } catch (error) { return failure(error) }
}
