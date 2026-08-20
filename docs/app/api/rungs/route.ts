/**
 * The models on the graph.
 *
 * Reading is open; writing needs the shared token, because whoever can write
 * here decides what every omni install resolves to.
 */

import { NextRequest, NextResponse } from "next/server"

import { add, configured, entries, plant, remove } from "../../../lib/db"
import { provider } from "../../../lib/providers"

export const dynamic = "force-dynamic"

function allowed(request: NextRequest): boolean {
  const token = process.env.ADMIN_TOKEN
  return Boolean(token) && request.headers.get("x-omni-token") === token
}

function refuse(why: string, status = 400) {
  return NextResponse.json({ error: why }, { status })
}

export async function GET() {
  return NextResponse.json({ live: configured, entries: await entries() })
}

export async function POST(request: NextRequest) {
  if (!allowed(request)) return refuse("wrong or missing token", 401)
  if (!configured) return refuse("no database configured on this deployment", 503)

  const body = await request.json().catch(() => null)
  if (!body) return refuse("expected a json body")
  if (body.action === "seed") return NextResponse.json({ planted: await plant() })

  const known = provider(String(body.provider ?? ""))
  if (!known) return refuse("pick one of the three providers")

  const model = String(body.model ?? "").trim()
  if (!model) return refuse("a model name is required")

  const effort = String(body.effort ?? "")
  if (!known.efforts.includes(effort)) {
    return refuse(`${known.label} does not take the effort "${effort}"`)
  }

  const number = (value: unknown) =>
    value === "" || value === null || value === undefined ? null : Number(value)
  const score = number(body.score)
  const price = number(body.price)
  if ((score !== null && !isFinite(score)) || (price !== null && !isFinite(price))) {
    return refuse("score and cost have to be numbers, or left blank")
  }
  if (price !== null && price < 0) return refuse("cost cannot be negative")

  await add({ provider: known.id, model, effort, score, price, note: body.note ?? null })
  return NextResponse.json({ ok: true })
}

export async function DELETE(request: NextRequest) {
  if (!allowed(request)) return refuse("wrong or missing token", 401)
  if (!configured) return refuse("no database configured on this deployment", 503)
  const id = Number(request.nextUrl.searchParams.get("id"))
  if (!id) return refuse("which one?")
  await remove(id)
  return NextResponse.json({ ok: true })
}
