/**
 * The model catalogue, borrowed from litellm.
 *
 * Only a naming aid: it makes the picker searchable instead of a blank box.
 * What actually reaches the CLI is whatever string is saved, which is why the
 * field stays free text — a CLI-only slug like `gpt-5.6-sol` or
 * `gemini-3.7-flash-high` is not in anybody's catalogue.
 */

const SOURCE =
  "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"

/** litellm provider ids for the vendors our three CLIs can reach. */
const VENDORS = new Set(["anthropic", "openai", "gemini"])

export interface Model {
  name: string
  vendor: string
  context: number | null
  reasoning: boolean
}

let cache: { at: number; models: Model[] } | null = null
const TTL = 60 * 60 * 1000

export async function models(): Promise<Model[]> {
  if (cache && Date.now() - cache.at < TTL) return cache.models
  // 2.3MB, past what the framework will cache — the route above caches the
  // small filtered answer instead, and this module holds one copy per instance.
  const response = await fetch(SOURCE, { cache: "no-store" })
  if (!response.ok) return cache?.models ?? []
  const raw = (await response.json()) as Record<string, Record<string, unknown>>
  const found: Model[] = []
  for (const [name, spec] of Object.entries(raw)) {
    if (!spec || typeof spec !== "object") continue
    const vendor = String(spec.litellm_provider ?? "")
    if (!VENDORS.has(vendor) || spec.mode !== "chat") continue
    found.push({
      name,
      vendor,
      context: typeof spec.max_input_tokens === "number" ? spec.max_input_tokens : null,
      reasoning: spec.supports_reasoning === true,
    })
  }
  found.sort((a, b) => a.vendor.localeCompare(b.vendor) || a.name.localeCompare(b.name))
  cache = { at: Date.now(), models: found }
  return found
}
