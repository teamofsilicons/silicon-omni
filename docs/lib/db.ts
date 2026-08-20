/**
 * The one table behind the dial.
 *
 * Any Postgres will do. On Vercel, add a Neon store and the connection string
 * arrives on its own. With no database configured the site still runs and
 * serves the seed, so a fresh clone is never broken — it just cannot be edited.
 */

import { neon } from "@neondatabase/serverless"

import seed from "../data/seed.json"
import type { Entry } from "./dial"

const URL = process.env.DATABASE_URL || process.env.POSTGRES_URL || ""

export const configured = Boolean(URL)

const sql = configured ? neon(URL) : null

let ready = false

async function ensure() {
  if (!sql || ready) return
  await sql`
    CREATE TABLE IF NOT EXISTS rungs (
      id       SERIAL PRIMARY KEY,
      provider TEXT NOT NULL,
      model    TEXT NOT NULL,
      effort   TEXT NOT NULL DEFAULT '',
      score    DOUBLE PRECISION,
      price    DOUBLE PRECISION,
      note     TEXT,
      added    TIMESTAMPTZ NOT NULL DEFAULT now(),
      UNIQUE (provider, model, effort)
    )`
  ready = true
}

/** Everything on the graph. Falls back to the shipped seed when there is no database. */
export async function entries(): Promise<Entry[]> {
  if (!sql) return seed.entries as Entry[]
  await ensure()
  const rows = await sql`
    SELECT id, provider, model, effort, score, price, note
    FROM rungs ORDER BY score DESC NULLS LAST, price ASC`
  return rows as unknown as Entry[]
}

export async function add(entry: Entry): Promise<void> {
  if (!sql) throw new Error("no database configured")
  await ensure()
  await sql`
    INSERT INTO rungs (provider, model, effort, score, price, note)
    VALUES (${entry.provider}, ${entry.model}, ${entry.effort}, ${entry.score}, ${entry.price}, ${entry.note ?? null})
    ON CONFLICT (provider, model, effort) DO UPDATE
      SET score = EXCLUDED.score, price = EXCLUDED.price, note = EXCLUDED.note`
}

export async function remove(id: number): Promise<void> {
  if (!sql) throw new Error("no database configured")
  await ensure()
  await sql`DELETE FROM rungs WHERE id = ${id}`
}

/** Copy the shipped seed into an empty table, so a new deploy starts useful. */
export async function plant(): Promise<number> {
  if (!sql) throw new Error("no database configured")
  await ensure()
  const [{ count }] = (await sql`SELECT count(*)::int AS count FROM rungs`) as { count: number }[]
  if (count > 0) return 0
  for (const entry of seed.entries as Entry[]) await add(entry)
  return (seed.entries as Entry[]).length
}
