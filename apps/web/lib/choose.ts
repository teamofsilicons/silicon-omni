/**
 * Turning a pile of models into one answer.
 *
 * There are three ways to ask, and they exist because no one of them is
 * enough on its own:
 *
 *   a key       chat.model("code")            — a shortlist chosen by hand
 *   a number    chat.model(intelligence=7)    — the 0-10 dial
 *   a model     chat.model(model=…, effort=…) — you already know
 *
 * The keys are curated on purpose. An earlier version of this file scored the
 * board on three normalised axes and took a weighted sum, and it kept
 * producing answers that were defensible and wrong: three models inside one
 * percent of each other and a hundred Elo apart, and a winner that moved when
 * a normalisation constant did. Ranking is not the same as judgement, so the
 * judgement is written down instead, in `keys` in models.json, one shortlist
 * per kind of work, best first.
 *
 * This is the only place the answer is worked out. The library does none of
 * it: it says what it wants and hands the strings it gets back to a CLI.
 */

import { canRunFast, provider } from "./providers"

export interface Pick {
  provider: string
  model: string
  effort: string
  /** run the model hot, where the CLI has such a thing */
  fast: boolean
}

export interface Model extends Pick {
  scores: Record<string, number>
  price: number
  secs: number
}

/** A row of the database. */
export interface Entry {
  provider: string
  model: string
  status?: "current" | "legacy" | "restricted"
  effort?: string
  fast?: boolean
  /** Recommend this configuration only when the caller reports the model as available. */
  requires_availability?: boolean
  scores?: Record<string, number | null> | null
  price?: number | null
  secs?: number | null
}

/** One curated shortlist: what it is for, and who answers it, best first. */
export interface Key {
  says: string
  picks: Entry[]
}

export type AvailableModels = Record<string, readonly string[]>

function eligible(entry: Entry, availableModels: AvailableModels): boolean {
  return !entry.requires_availability ||
    (Object.hasOwn(availableModels, entry.provider) && availableModels[entry.provider].includes(entry.model))
}

export const LEVELS = 11

function number(value: unknown): number | null {
  if (value === null || value === undefined || value === "") return null
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

/** Can this actually be run: a provider we drive, at an effort it accepts. */
export function runnable(entry: Entry): Pick | null {
  const known = provider(entry.provider)
  const effort = entry.effort ?? ""
  if (!known || !known.efforts.includes(effort)) return null
  return {
    provider: known.id,
    model: entry.model,
    effort,
    // asking a CLI with no fast mode to run hot is ignored, not refused
    fast: Boolean(entry.fast) && canRunFast(known.id, entry.model),
  }
}

/** The rows that can be plotted on `bench` as well as run. */
export function plottable(entries: Entry[], bench: string): Model[] {
  const out: Model[] = []
  for (const entry of entries) {
    const can = runnable(entry)
    if (!can) continue
    const score = number(entry.scores?.[bench])
    const price = number(entry.price)
    const secs = number(entry.secs)
    if (score === null || price === null || secs === null) continue
    out.push({ ...can, scores: entry.scores as Record<string, number>, price, secs })
  }
  return out
}

/** Rows the database holds that this benchmark cannot use, and why. */
export function unusable(entries: Entry[], bench: string): { entry: Entry; why: string }[] {
  const out: { entry: Entry; why: string }[] = []
  for (const entry of entries) {
    const known = provider(entry.provider)
    const effort = entry.effort ?? ""
    if (entry.status === "legacy" || entry.status === "restricted")
      out.push({ entry, why: `${entry.status} model is excluded from automatic selection` })
    else if (!known) out.push({ entry, why: `no such provider "${entry.provider}"` })
    else if (!known.efforts.includes(effort))
      out.push({ entry, why: `${known.label} does not take the effort "${effort}"` })
    else if (number(entry.scores?.[bench]) === null)
      out.push({ entry, why: `not scored on "${bench}"` })
    else if (number(entry.price) === null || number(entry.secs) === null)
      out.push({ entry, why: "no cost or time measured yet" })
  }
  return out
}

/* ------------------------------------------------------------------ by key */

export interface KeyAnswer {
  how: "key"
  key: string
  says: string
  /** the first pick whose provider is signed in */
  pick: Pick | null
  /** the whole shortlist, best first, with what was available marked */
  shortlist: (Pick & { available: boolean; requires_availability?: boolean })[]
}

/**
 * The shortlist for a kind of work, resolved against what is signed in.
 *
 * Left to right is the order somebody decided, so the first entry whose
 * provider is available wins. Losing a vendor moves you down the list rather
 * than into an argument about weights.
 */
export function byKey(keys: Record<string, Key>, key: string, providers: string[], availableModels: AvailableModels = {}): KeyAnswer | null {
  if (!Object.hasOwn(keys, key)) return null
  const found = keys[key]
  if (!found) return null
  const shortlist = found.picks.flatMap(entry => {
    const pick = runnable(entry)
    if (!pick) return []
    const requires = entry.requires_availability === true
    return [{ ...pick, ...(requires ? { requires_availability: true } : {}),
      available: providers.includes(pick.provider) && eligible(entry, availableModels),
    }]
  })
  return {
    how: "key",
    key,
    says: found.says,
    pick: shortlist.find((p) => p.available) ?? null,
    shortlist,
  }
}

/* --------------------------------------------------------------- by number */

/** Is `one` at least as good as `other` on both axes, and better on one? */
function beats(one: Model, other: Model, bench: string): boolean {
  const a = one.scores[bench]
  const b = other.scores[bench]
  return a >= b && one.price <= other.price && (a > b || one.price < other.price)
}

/**
 * The left edge of the board: a model earns a place when nothing else is both
 * better and cheaper. Best first.
 */
export function edge(models: Model[], bench: string): Model[] {
  const kept = models.filter((p) => !models.some((q) => beats(q, p, bench)))
  const seen = new Set<string>()
  const out: Model[] = []
  for (const point of [...kept].sort(
    (a, b) => b.scores[bench] - a.scores[bench] || a.price - b.price,
  )) {
    const spot = `${point.scores[bench]}/${point.price}`
    if (seen.has(spot)) continue // two models on one spot are one point
    seen.add(spot)
    out.push(point)
  }
  return out
}

export interface DialAnswer {
  how: "intelligence"
  basis: "measured"
  intelligence: number
  bench: string
  pick: Pick | null
  /** every rung, so a caller can see what the number means here */
  ladder: Record<string, Pick>
}

/** Spread the edge over 0-10, 10 at the top. */
export function ladderOf(walk: Model[]): Record<string, Pick> {
  if (!walk.length) return {}
  const steps = walk.length - 1
  const out: Record<string, Pick> = {}
  for (let level = 0; level < LEVELS; level++) {
    const m = walk[Math.round(((LEVELS - 1 - level) * steps) / (LEVELS - 1))]
    out[String(level)] = { provider: m.provider, model: m.model, effort: m.effort, fast: m.fast }
  }
  return out
}

/**
 * The dial, kept because a single number is still the easiest thing to reach
 * for when you have no opinion beyond "better" or "cheaper".
 */
export function byIntelligence(
  entries: Entry[],
  providers: string[],
  bench: string,
  intelligence: number,
  availableModels: AvailableModels = {},
): DialAnswer {
  const current = entries.filter((entry) => entry.status !== "legacy" && entry.status !== "restricted" && eligible(entry, availableModels))
  const models = plottable(current, bench).filter((m) => providers.includes(m.provider))
  const ladder = ladderOf(edge(models, bench))
  const level = Math.max(0, Math.min(10, Math.round(intelligence)))
  return { how: "intelligence", basis: "measured", intelligence: level, bench, pick: ladder[String(level)] ?? null, ladder }
}

const CURATED_CATEGORIES = ["cost", "cost", "cost", "fast", "general", "general", "general", "code", "code", "research", "research"]

/** The default dial follows current editorial choices, without inventing scores. */
export function byCuratedIntelligence(keys: Record<string, Key>, providers: string[], intelligence: number, availableModels: AvailableModels = {}) {
  const level = Math.max(0, Math.min(10, Math.round(intelligence)))
  const ladder: Record<string, Pick | null> = Object.fromEntries(
    CURATED_CATEGORIES.map((key, index) => [String(index), byKey(keys, key, providers, availableModels)?.pick ?? null]),
  )
  return {
    how: "intelligence" as const,
    basis: "curated" as const,
    intelligence: level,
    category: CURATED_CATEGORIES[level],
    pick: ladder[String(level)],
    ladder,
  }
}

/* ---------------------------------------------------------------- by name */

export interface NamedAnswer {
  how: "model"
  pick: Pick | null
  /** why it was refused, when it was */
  why?: string
}

/**
 * You already know what you want.
 *
 * Checked but not second-guessed: the model string is passed to the CLI
 * verbatim, so a model released this morning works without anything here
 * knowing about it. Only the provider and the effort are validated, because
 * those are the two things that would make the command fail to run at all.
 */
export function byName(
  providers: string[],
  asked: { provider?: string; model: string; effort?: string; fast?: boolean },
): NamedAnswer {
  const effort = asked.effort ?? ""
  const candidates = asked.provider ? [asked.provider] : providers
  for (const id of candidates) {
    const known = provider(id)
    if (!known) continue
    if (!providers.includes(known.id))
      return { how: "model", pick: null, why: `${known.label} is not signed in` }
    if (!known.efforts.includes(effort))
      return { how: "model", pick: null, why: `${known.label} does not take the effort "${effort}"` }
    if (asked.fast && known.id === "claude-code-cli" && !canRunFast(known.id, asked.model))
      return { how: "model", pick: null, why: `Claude fast mode does not support ${asked.model}; use normal speed or a supported Opus model` }
    return {
      how: "model",
      pick: { provider: known.id, model: asked.model, effort, fast: Boolean(asked.fast) && canRunFast(known.id, asked.model) },
    }
  }
  return { how: "model", pick: null, why: `no provider named "${asked.provider}"` }
}
