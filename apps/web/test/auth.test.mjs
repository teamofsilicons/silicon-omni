import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { test } from "node:test"

const require = createRequire(import.meta.url)
const ts = require("typescript")
const source = readFileSync(new URL("../lib/auth.ts", import.meta.url), "utf8")
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText
const auth = await import(`data:text/javascript,${encodeURIComponent(compiled)}`)
const ADMIN = "https://admin.omni.teamofsilicons.com"
const API = "https://backend.omni.teamofsilicons.com"
const ISSUER = "https://accounts.teamofsilicons.com"
const UUID = "0080c488-a9e9-4c22-aa91-1c7a639f1d7b"
const OTHER = "7e3bd279-da39-417f-b6dc-8ad0924990e4"

function setup(t, overrides = {}) {
  const oldFetch = globalThis.fetch
  const previous = Object.fromEntries(["SILICON_APP_SECRET", "OMNI_ADMIN_IDS", "OMNI_ADMIN_ORIGIN", "SILICON_ACCOUNTS_URL"].map(k => [k, process.env[k]]))
  Object.assign(process.env, { SILICON_APP_SECRET: "test-secret-only-012345678901234567890123456789", OMNI_ADMIN_IDS: UUID, OMNI_ADMIN_ORIGIN: ADMIN, SILICON_ACCOUNTS_URL: ISSUER })
  const calls = []
  const claims = { active: true, sub: UUID, aud: "omni", client_id: "omni", kind: "silicon", id: "si:tos", token_type: "access_token", exp: Math.floor(Date.now() / 1000) + 1800, iss: ISSUER, ...overrides }
  globalThis.fetch = async (url, options) => {
    calls.push({ url, options, body: JSON.parse(options.body) })
    if (url.endsWith("/introspect")) return Response.json(claims)
    if (url.endsWith("/revoke")) { claims.active = false; return Response.json({}) }
    if (url.endsWith("/device/authorize")) return Response.json({ device_code: "sad_private", user_code: "ABCD-EFGH", verification_uri: `${ISSUER}/device`, verification_uri_complete: `${ISSUER}/device?code=ABCD-EFGH`, expires_in: 600, interval: 5 })
    return Response.json({ token_type: "Bearer", access_token: "access_token_that_must_stay_private", refresh_token: "never_store_refresh_token", expires_in: 1800, account: { uuid: UUID, id: "si:tos", kind: "silicon" } })
  }
  t.after(() => {
    globalThis.fetch = oldFetch
    for (const [key, value] of Object.entries(previous)) value === undefined ? delete process.env[key] : process.env[key] = value
  })
  return { calls, claims }
}

function request(path, { method = "GET", origin, cookie, token, body, host = API } = {}) {
  const headers = new Headers()
  if (origin) headers.set("Origin", origin)
  if (cookie) headers.set("Cookie", cookie)
  if (token) headers.set("Authorization", `Bearer ${token}`)
  if (body) headers.set("Content-Type", "application/json")
  return new Request(`${host}${path}`, { method, headers, ...(body ? { body: JSON.stringify(body) } : {}) })
}

async function login() {
  const response = await auth.loginWithGrant(request("/api/auth/slt", { method: "POST", body: { slt: "slt_once" } }), "slt")
  return response.json()
}

const code = expected => error => error instanceof auth.AuthError && error.code === expected

test("CLI session is opaque, short-lived, and checked remotely before every admin request", async t => {
  const { calls, claims } = setup(t)
  const result = await login()
  assert.equal(result.account.uuid, UUID)
  assert.equal(result.is_admin, true)
  assert.ok(Date.parse(result.expires_at) - Date.now() <= 1800_000)
  assert.ok(!JSON.stringify(result).includes("access_token_that"))
  assert.ok(!JSON.stringify(result).includes("never_store"))
  const req = request("/api/admin/models", { method: "PUT", token: result.session_token })
  assert.equal((await auth.requireAdmin(req)).account.id, "si:tos")
  claims.active = false
  await assert.rejects(auth.requireAdmin(req), code("session_expired"))
  assert.deepEqual(await auth.getSession(req), { authenticated: false })
  assert.equal(calls[0].options.redirect, "error")
  assert.equal(calls[0].options.cache, "no-store")
  assert.ok(calls[0].options.headers.Authorization.startsWith("Basic "))
})

test("only immutable allowlisted UUIDs authorize writes", async t => {
  setup(t)
  const result = await login()
  process.env.OMNI_ADMIN_IDS = `si:tos,${OTHER}`
  await assert.rejects(auth.requireAdmin(request("/api/admin/models", { method: "PUT", token: result.session_token })), code("admin_required"))
  process.env.OMNI_ADMIN_IDS = ""
  assert.equal((await auth.getSession(request("/api/auth/session", { token: result.session_token }))).is_admin, false)
})

test("tampered and expired sessions cannot authenticate", async t => {
  setup(t)
  const result = await login()
  const token = result.session_token
  const changed = (token[0] === "a" ? "b" : "a") + token.slice(1)
  assert.deepEqual(await auth.getSession(request("/api/auth/session", { token: changed })), { authenticated: false })
  const now = Date.now
  Date.now = () => now() + 1801_000
  try { assert.deepEqual(await auth.getSession(request("/api/auth/session", { token })), { authenticated: false }) }
  finally { Date.now = now }
})

test("wrong audience, client, token type or immutable identity cannot create a session", async t => {
  const { claims } = setup(t)
  for (const [key, bad] of [["aud", "another-app"], ["client_id", "another-app"], ["token_type", "refresh_token"], ["sub", OTHER], ["kind", "service"], ["iss", "https://evil.example"]]) {
    const before = claims[key]
    claims[key] = bad
    await assert.rejects(login(), code("invalid_session"))
    claims[key] = before
  }
})

test("browser login sets a host-only secure HttpOnly cookie and exposes no bearer", async t => {
  setup(t)
  const response = await auth.loginWithGrant(request("/api/auth/slt", { host: ADMIN, method: "POST", origin: ADMIN, body: { slt: "slt_once" } }), "slt")
  const result = await response.json()
  assert.equal(result.authenticated, true)
  assert.equal(result.session_token, undefined)
  const cookie = response.headers.get("set-cookie")
  assert.match(cookie, /^__Host-omni_session=/)
  assert.match(cookie, /HttpOnly; SameSite=Lax/)
  assert.match(cookie, /; Secure/)
  assert.ok(!cookie.includes("Domain="))
  const selected = cookie.split(";")[0]
  await assert.rejects(auth.requireAdmin(request("/api/admin/models", { host: ADMIN, method: "PUT", cookie: selected })), code("invalid_origin"))
  await assert.rejects(auth.requireAdmin(request("/api/admin/models", { host: ADMIN, method: "PUT", cookie: selected, origin: "https://evil.example" })), code("invalid_origin"))
  assert.equal((await auth.requireAdmin(request("/api/admin/models", { host: ADMIN, method: "PUT", cookie: selected, origin: ADMIN }))).is_admin, true)
  assert.deepEqual(await auth.getSession(request("/api/auth/session", { cookie: selected })), { authenticated: false })
})

test("cross-origin login and foreign return paths fail before any token exchange", async t => {
  const { calls } = setup(t)
  await assert.rejects(auth.loginWithGrant(request("/api/auth/slt", { host: ADMIN, method: "POST", origin: "https://evil.example", body: { slt: "slt_once" } }), "slt"), code("invalid_origin"))
  for (const path of ["//evil.example", "https://evil.example", "/\\evil.example", "/\n/evil.example"]) assert.throws(() => auth.safeReturnPath(path), code("invalid_return"))
  assert.equal(auth.safeReturnPath("/admin?tab=models"), "/admin?tab=models")
  assert.equal(calls.length, 0)
})

test("browser callback binds state and PKCE to a private expiring cookie", async t => {
  const { calls } = setup(t)
  const started = auth.startBrowser(request("/auth/login?return_to=/admin", { host: ADMIN }))
  const destination = new URL(started.headers.get("location"))
  assert.equal(destination.origin, ISSUER)
  assert.equal(destination.searchParams.get("code_challenge_method"), "S256")
  assert.equal(destination.searchParams.get("redirect_uri"), `${ADMIN}/auth/callback`)
  assert.equal(destination.searchParams.has("code_verifier"), false)
  const cookie = started.headers.get("set-cookie").split(";")[0]
  const bad = await auth.completeBrowser(request("/auth/callback?code=sac_once&state=wrong", { host: ADMIN, cookie }))
  assert.equal(bad.status, 401)
  assert.equal(calls.length, 0)
  assert.match(bad.headers.get("set-cookie"), /Max-Age=0/)
  const state = destination.searchParams.get("state")
  const completed = await auth.completeBrowser(request(`/auth/callback?code=sac_once&state=${state}`, { host: ADMIN, cookie }))
  assert.equal(completed.status, 303)
  assert.equal(completed.headers.get("location"), `${ADMIN}/admin`)
  assert.equal(calls[0].body.grant_type, "authorization_code")
  assert.equal(typeof calls[0].body.code_verifier, "string")
})

test("Accounts outages fail closed without exposing tokens or provider details", async t => {
  setup(t)
  const result = await login()
  globalThis.fetch = async () => { throw new Error("secret upstream debug text") }
  await assert.rejects(auth.requireAdmin(request("/api/admin/models", { method: "PUT", token: result.session_token })), code("accounts_unavailable"))
  const response = auth.authError(new Error("secret upstream debug text"))
  assert.ok(!(await response.text()).includes("secret upstream"))
})

test("device login handles pending status and only returns approved fields", async t => {
  const { calls } = setup(t)
  const response = await auth.startDevice(request("/api/auth/device", { method: "POST" }))
  const device = await response.json()
  assert.equal(device.verification_uri, `${ISSUER}/device`)
  assert.equal(device.interval, 5)
  assert.equal(calls[0].body.client_id, "omni")
  globalThis.fetch = async () => Response.json({ error: "authorization_pending", error_description: "private upstream details" }, { status: 400 })
  await assert.rejects(auth.loginWithGrant(request("/api/auth/token", { method: "POST", body: { device_code: "sad_private" } }), "device"), code("authorization_pending"))
})

test("logout revokes the Accounts token, ending copies of the same session", async t => {
  const { calls } = setup(t)
  const result = await login()
  const response = await auth.logout(request("/api/auth/logout", { method: "POST", token: result.session_token }))
  assert.deepEqual(await response.json(), { authenticated: false })
  assert.ok(calls.some(call => call.url.endsWith("/revoke")))
  assert.deepEqual(await auth.getSession(request("/api/auth/session", { token: result.session_token })), { authenticated: false })
})
