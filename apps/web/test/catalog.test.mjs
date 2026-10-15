import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { test } from "node:test"

const require = createRequire(import.meta.url)
const ts = require("typescript")
const dataUrl = js => `data:text/javascript,${encodeURIComponent(js)}`
const transpile = path => ts.transpileModule(readFileSync(new URL(path, import.meta.url), "utf8"), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText
const providers = dataUrl(transpile("../lib/providers.ts"))
const catalogUrl = dataUrl(transpile("../lib/catalog.ts").replace('from "./providers"', `from "${providers}"`))
const { validateCatalog, modelIdentity } = await import(catalogUrl)
const seed = () => JSON.parse(readFileSync(new URL("../models.json", import.meta.url), "utf8"))

test("current catalog is valid and keeps unmeasured values distinct from zero", () => {
  const catalog = validateCatalog(seed())
  assert.equal(Object.keys(catalog.keys).length, 6)
  assert.ok(catalog.models.some(m => m.price === null && m.secs === null))
  assert.ok(catalog.models.some(m => m.status === "legacy"))
})

test("duplicate model configurations are rejected instead of ambiguously ranked", () => {
  const catalog = seed()
  catalog.models.push({ ...catalog.models[0] })
  assert.throws(() => validateCatalog(catalog), /Duplicate model configuration/)
})

test("recommendations must reference present, current configurations", () => {
  const catalog = seed()
  const entry = catalog.models.find(m => modelIdentity(m) === modelIdentity(catalog.keys.code.picks[0]))
  entry.status = "legacy"
  assert.throws(() => validateCatalog(catalog), /legacy or restricted/)
  entry.status = "restricted"
  assert.throws(() => validateCatalog(catalog), /legacy or restricted/)
  entry.status = "current"
  catalog.keys.code.picks[0].model = "missing-from-the-catalog"
  assert.throws(() => validateCatalog(catalog), /missing from the catalog/)
})

test("malformed status and recommendation fields cannot bypass lifecycle or provider rules", () => {
  const catalog = seed()
  catalog.models[0].status = ["legacy"]
  assert.throws(() => validateCatalog(catalog), /Status must/)
  const other = seed()
  other.keys.code.picks[0].effort = [other.keys.code.picks[0].effort]
  assert.throws(() => validateCatalog(other), /text provider, model, and effort/)
})

test("invalid provider effort, negative cost, and non-finite scores are rejected", () => {
  const changes = [
    [m => m.provider = "unknown-provider", /supported provider/],
    [m => m.effort = "impossible", /Unsupported effort/],
    [m => m.price = -1, /nonnegative/],
    [m => m.secs = "fast", /nonnegative/],
    [m => m.scores = { benchmark: Infinity }, /finite numbers/],
  ]
  for (const [change, pattern] of changes) {
    const catalog = seed()
    change(catalog.models[0])
    assert.throws(() => validateCatalog(catalog), pattern)
  }
})

test("all six categories and their nonempty shortlists are required", () => {
  const catalog = seed()
  delete catalog.keys.fast
  assert.throws(() => validateCatalog(catalog), /six categories/)
  const other = seed()
  other.keys.cost.picks = []
  assert.throws(() => validateCatalog(other), /at least one recommendation/)
})

const authMock = dataUrl(`
export class AuthError extends Error { constructor(status,code,message){super(message);this.status=status;this.code=code} }
export function authJson(value,status=200){return Response.json(value,{status,headers:{'Cache-Control':'no-store'}})}
export function authError(error){return authJson({error:{code:error.code,message:error.message}},error.status)}
export async function requireAdmin(){if(!globalThis.omniCatalogRouteTest.authorized)throw new AuthError(401,'sign_in_required','Sign in.');return {account:{uuid:'immutable-admin-uuid'}}}
`)
const storeMock = dataUrl(`
export class CatalogConflict extends Error {}
export async function readCatalog(){const s=globalThis.omniCatalogRouteTest;s.reads++;if(s.storageError)throw new Error('private bucket arn and secret');return s.result}
export async function writeCatalog(catalog,revision,actor){const s=globalThis.omniCatalogRouteTest;if(s.conflict)throw new CatalogConflict();s.write={catalog,revision,actor};return {catalog,revision:'"def456"',writable:true}}
`)
const route = await import(dataUrl(transpile("../app/api/admin/catalog/route.ts")
  .replace('from "@/lib/auth"', `from "${authMock}"`)
  .replace('from "@/lib/catalog"', `from "${catalogUrl}"`)
  .replace('from "@/lib/store"', `from "${storeMock}"`)))

function setupRoute(t) {
  const state = { authorized: true, reads: 0, result: { catalog: seed(), revision: '"abc123"', writable: true } }
  globalThis.omniCatalogRouteTest = state
  t.after(() => delete globalThis.omniCatalogRouteTest)
  return state
}

function put(body, contentType = "application/json") {
  return new Request("https://admin.omni.teamofsilicons.com/api/admin/catalog", { method: "PUT", headers: { "Content-Type": contentType }, body: JSON.stringify(body) })
}

test("admin catalog authorization runs before reading or mutating storage", async t => {
  const state = setupRoute(t)
  state.authorized = false
  assert.equal((await route.GET(new Request("https://admin.omni.teamofsilicons.com/api/admin/catalog"))).status, 401)
  assert.equal((await route.PUT(put({ catalog: seed(), revision: '"abc123"' }))).status, 401)
  assert.equal(state.reads, 0)
  assert.equal(state.write, undefined)
})

test("catalog publish records authenticated UUID and honors optimistic revision conflicts", async t => {
  const state = setupRoute(t)
  const response = await route.PUT(put({ catalog: seed(), revision: '"abc123"', actor: "spoofed-user" }))
  assert.equal(response.status, 200)
  assert.equal(response.headers.get("cache-control"), "no-store")
  assert.equal(state.write.actor, "immutable-admin-uuid")
  assert.equal(state.write.revision, '"abc123"')
  state.conflict = true
  assert.equal((await route.PUT(put({ catalog: seed(), revision: '"abc123"' }))).status, 409)
})

test("invalid catalog payloads and revisions never write storage", async t => {
  const state = setupRoute(t)
  assert.equal((await route.PUT(put({ catalog: {}, revision: '"abc123"' }))).status, 400)
  assert.equal((await route.PUT(put({ catalog: seed(), revision: "stale" }))).status, 400)
  assert.equal((await route.PUT(put({}, "text/plain"))).status, 415)
  assert.equal(state.write, undefined)
})

test("streamed payloads over 1 MiB are rejected without trusting Content-Length", async t => {
  const state = setupRoute(t)
  const chunks = [new Uint8Array(600_000), new Uint8Array(600_000)]
  let cancelled = false
  const stream = new ReadableStream({ pull(controller) { const chunk = chunks.shift(); if (chunk) controller.enqueue(chunk) }, cancel() { cancelled = true } })
  const req = new Request("https://admin.omni.teamofsilicons.com/api/admin/catalog", { method: "PUT", headers: { "Content-Type": "application/json" }, body: stream, duplex: "half" })
  assert.equal((await route.PUT(req)).status, 413)
  assert.equal(state.write, undefined)
  assert.equal(cancelled, true)
})

test("storage failure details remain private", async t => {
  const state = setupRoute(t)
  state.storageError = true
  const response = await route.GET(new Request("https://admin.omni.teamofsilicons.com/api/admin/catalog"))
  assert.equal(response.status, 503)
  assert.ok(!(await response.text()).includes("private bucket"))
})
