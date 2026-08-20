/**
 * The models the dial is built from, read straight out of the repo.
 *
 * There is no database. The list lives at `docs/data/models.json` on the
 * default branch, and the site reads it over raw.githubusercontent at request
 * time — so editing that file and committing is the whole publishing flow. No
 * release, no redeploy, no admin password to lose.
 *
 * The copy bundled at build time is the backstop for when GitHub is unreachable.
 */

import bundled from "../data/models.json"
import type { Entry } from "./dial"

const REPO = process.env.OMNI_REPO || "teamofsilicons/silicon-omni"
// raw.githubusercontent will not resolve HEAD, so this has to name a real
// branch. Set OMNI_BRANCH to read the list from somewhere other than main.
const BRANCH = process.env.OMNI_BRANCH || "main"
const PATH = "docs/data/models.json"

export const SOURCE = `https://raw.githubusercontent.com/${REPO}/${BRANCH}/${PATH}`
export const EDIT = `https://github.com/${REPO}/edit/${BRANCH}/${PATH}`
export const VIEW = `https://github.com/${REPO}/blob/${BRANCH}/${PATH}`

export interface Graph {
  models: Entry[]
  source: Record<string, string>
  caveats: string[]
  /** false when we fell back to the copy compiled into this deployment */
  fresh: boolean
}

function shape(raw: unknown, fresh: boolean): Graph | null {
  if (!raw || typeof raw !== "object") return null
  const doc = raw as Record<string, unknown>
  const models = doc.models
  if (!Array.isArray(models)) return null
  return {
    models: models.filter((m) => m && typeof m === "object" && m.provider && m.model) as Entry[],
    source: (doc.source ?? {}) as Record<string, string>,
    caveats: (doc.caveats ?? []) as string[],
    fresh,
  }
}

const FALLBACK = shape(bundled, false) as Graph

export async function graph(): Promise<Graph> {
  try {
    // Short cache: an edit should show up in minutes, not on the next deploy.
    const response = await fetch(SOURCE, { next: { revalidate: 60 } })
    if (!response.ok) return FALLBACK
    return shape(await response.json(), true) ?? FALLBACK
  } catch {
    return FALLBACK
  }
}
