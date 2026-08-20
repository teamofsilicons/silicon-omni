/**
 * The three CLIs omni drives, and what each accepts.
 *
 * Effort is a per-provider thing, not a per-model thing: every model a given
 * CLI can run takes the same effort words, so these are hard-coded rather than
 * looked up. An empty effort means the flag is not passed at all, which some
 * models require and which Antigravity needs whenever the slug already carries
 * the effort (`gemini-3.7-flash-high`).
 */

export type ProviderId = "claude" | "openai" | "google"

export interface Provider {
  id: ProviderId
  label: string
  cli: string
  /** litellm's own provider ids for the models this CLI usually serves. */
  vendors: string[]
  /** every effort string the CLI accepts, weakest first. "" means omit the flag. */
  efforts: string[]
}

export const PROVIDERS: Provider[] = [
  {
    id: "claude",
    label: "Claude Code",
    cli: "claude --model <model> --effort <effort>",
    vendors: ["anthropic"],
    efforts: ["", "low", "medium", "high", "xhigh", "max"],
  },
  {
    id: "openai",
    label: "Codex",
    cli: "codex app-server · turn/start { model, effort }",
    vendors: ["openai"],
    efforts: ["", "low", "medium", "high", "xhigh", "max", "ultra"],
  },
  {
    id: "google",
    label: "Antigravity",
    cli: "agy --model <model> --effort <effort>",
    vendors: ["gemini", "anthropic", "openai"],
    efforts: ["", "low", "medium", "high"],
  },
]

export const PROVIDER_IDS = PROVIDERS.map((p) => p.id)

export function provider(id: string): Provider | undefined {
  return PROVIDERS.find((p) => p.id === id)
}

/** Every non-empty combination of providers, named the way omni asks for them. */
export function combinations(ids: ProviderId[] = PROVIDER_IDS as ProviderId[]): string[] {
  const out: string[] = []
  for (let mask = 1; mask < 1 << ids.length; mask++) {
    out.push(ids.filter((_, i) => mask & (1 << i)).sort().join("+"))
  }
  return out
}

/** One dial per set of providers, named the same way on both sides. */
export function key(providers: string[]): string {
  return [...new Set(providers)].sort().join("+")
}
