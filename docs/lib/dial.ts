/**
 * Turning a pile of models into a 0-10 dial.
 *
 * Every model is a point on a graph: how good it is, and what it costs. Only
 * the left edge of that graph becomes a dial — a model earns a level if nothing
 * else is both better and cheaper. Level 10 is the top of the edge, and the
 * walk goes down and to the left, so a step down is always a real saving and
 * never a sideways move.
 *
 * This is the same calculation `tools/build_ladder.py` does offline in the
 * repo. Here it runs per request, over whatever is in the database.
 */

import { key } from "./providers"

export interface Rung {
  provider: string
  model: string
  effort: string
  score: number
  price: number
}

/** A row on the graph. Without both numbers it cannot be plotted, which is
 *  allowed on purpose: you can record a model before you have benchmarked it. */
export interface Entry {
  id?: number
  provider: string
  model: string
  effort: string
  score: number | null
  price: number | null
  note?: string | null
}

export const LEVELS = 11

/** Is `one` at least as good as `other` on both axes, and better on one? */
function beats(one: Rung, other: Rung): boolean {
  return (
    one.score >= other.score &&
    one.price <= other.price &&
    (one.score > other.score || one.price < other.price)
  )
}

/** The left edge of the graph, best first. */
export function edge(points: Rung[]): Rung[] {
  const kept = points.filter((p) => !points.some((q) => beats(q, p)))
  const seen = new Set<string>()
  const out: Rung[] = []
  for (const point of [...kept].sort((a, b) => b.score - a.score || a.price - b.price)) {
    const spot = `${point.score}/${point.price}`
    if (!seen.has(spot)) {
      seen.add(spot) // two models on one spot are one point
      out.push(point)
    }
  }
  return out
}

/** Spread the edge over levels 0-10, 10 at the top. */
export function dial(points: Rung[]): Record<string, Rung> {
  if (!points.length) return {}
  const steps = points.length - 1
  const out: Record<string, Rung> = {}
  for (let level = 0; level < LEVELS; level++) {
    out[String(level)] = points[Math.round(((LEVELS - 1 - level) * steps) / (LEVELS - 1))]
  }
  return out
}

/** Only entries with both numbers can be placed on the graph. */
export function plottable(entries: Entry[]): Rung[] {
  return entries
    .filter((e) => e.score !== null && e.price !== null)
    .map((e) => ({
      provider: e.provider,
      model: e.model,
      effort: e.effort ?? "",
      score: Number(e.score),
      price: Number(e.price),
    }))
}

/** The dial for one set of providers. */
export function dialFor(entries: Entry[], providers: string[]): Record<string, Rung> {
  const mine = plottable(entries).filter((p) => providers.includes(p.provider))
  return dial(edge(mine))
}

/** Every dial, keyed the way omni asks for them. */
export function allDials(entries: Entry[], combos: string[]): Record<string, Record<string, Rung>> {
  const out: Record<string, Record<string, Rung>> = {}
  for (const combo of combos) out[key(combo.split("+"))] = dialFor(entries, combo.split("+"))
  return out
}
