import type { Entry, Key } from "./choose"
import { PROVIDERS } from "./providers"

export const CATEGORIES = ["fast", "code", "design", "research", "cost", "general"] as const
export interface CatalogEntry extends Entry {
  status?: "current" | "legacy" | "restricted"
  notes?: string
  [key: string]: unknown
}
export interface Catalog {
  models: CatalogEntry[]
  keys: Record<string, Key>
  benches: Record<string, { label: string; short: string; url: string }>
  source: Record<string, string>
  caveats: string[]
  updated_at?: string
  updated_by?: string
  [key: string]: unknown
}
export function modelIdentity(m: Entry): string {
  return `${m.provider}/${m.model}/${m.effort ?? ""}`
}
function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}
/** Reject a broken catalog before it becomes the recommendation for every client. */
export function validateCatalog(raw: unknown): Catalog {
  if (!object(raw) || !Array.isArray(raw.models) || raw.models.length < 1 || raw.models.length > 2000)
    throw new Error("Catalog must contain between 1 and 2000 model configurations.")
  const identities = new Map<string, CatalogEntry>()
  for (const value of raw.models) {
    if (!object(value)) throw new Error("Each model must be an object.")
    const p = PROVIDERS.find(p => p.id === value.provider)
    if (!p || typeof value.model !== "string" || !/^[a-zA-Z0-9][a-zA-Z0-9._:/-]{0,119}$/.test(value.model))
      throw new Error("Each model needs a supported provider and a valid model ID.")
    if (value.effort !== undefined && (typeof value.effort !== "string" || !p.efforts.includes(value.effort)))
      throw new Error(`Unsupported effort for ${value.model}.`)
    if (value.fast !== undefined && typeof value.fast !== "boolean") throw new Error("fast must be a boolean.")
    if (value.status !== undefined && (typeof value.status !== "string" || !["current", "legacy", "restricted"].includes(value.status)))
      throw new Error("Status must be current, legacy, or restricted.")
    for (const field of ["price", "secs"]) {
      const n = value[field]
      if (n !== undefined && n !== null && (typeof n !== "number" || !Number.isFinite(n) || n < 0))
        throw new Error(`${field} must be a nonnegative measurement or null.`)
    }
    if (value.scores !== undefined && value.scores !== null) {
      if (!object(value.scores) || Object.values(value.scores).some(n => n !== null && (typeof n !== "number" || !Number.isFinite(n))))
        throw new Error("Benchmark scores must be finite numbers or null.")
    }
    const entry = value as unknown as CatalogEntry
    const id = modelIdentity(entry)
    if (identities.has(id)) throw new Error(`Duplicate model configuration: ${id}.`)
    identities.set(id, entry)
  }
  if (!object(raw.keys) || Object.keys(raw.keys).sort().join() !== [...CATEGORIES].sort().join())
    throw new Error("Keep the six categories: fast, code, design, research, cost, general.")
  for (const [id, key] of Object.entries(raw.keys)) {
    if (!object(key) || typeof key.says !== "string" || !key.says.trim() || !Array.isArray(key.picks) || !key.picks.length)
      throw new Error(`${id} needs a description and at least one recommendation.`)
    for (const pick of key.picks) {
      if (!object(pick)) throw new Error(`${id} has an invalid recommendation.`)
      if (typeof pick.provider !== "string" || typeof pick.model !== "string" ||
          (pick.effort !== undefined && typeof pick.effort !== "string"))
        throw new Error(`${id} recommendations need text provider, model, and effort fields.`)
      const entry = identities.get(modelIdentity(pick as unknown as Entry))
      if (!entry) throw new Error(`${id} recommends a configuration missing from the catalog.`)
      if (entry.status === "legacy" || entry.status === "restricted")
        throw new Error(`${id} recommends a legacy or restricted model. Choose a current model.`)
      if (pick.fast !== undefined && typeof pick.fast !== "boolean") throw new Error("fast must be a boolean.")
    }
  }
  if (!object(raw.benches) || Object.values(raw.benches).some(b => !object(b) || ["label", "short", "url"].some(k => typeof b[k] !== "string")))
    throw new Error("Benchmark definitions must include label, short, and url.")
  if (!object(raw.source) || Object.values(raw.source).some(v => typeof v !== "string"))
    throw new Error("Source metadata must contain text values.")
  if (!Array.isArray(raw.caveats) || raw.caveats.some(c => typeof c !== "string"))
    throw new Error("Caveats must be a list of text notes.")
  return raw as unknown as Catalog
}
