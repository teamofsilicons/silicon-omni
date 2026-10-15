import { createCipheriv, createDecipheriv, createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto"

const APP_ID = "omni"
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const SESSION_SECONDS = 30 * 60
type Json = Record<string, unknown>

export class AuthError extends Error {
  constructor(public status: number, public code: string, message: string) { super(message) }
}

function configuredUrl(value: string): URL {
  const url = new URL(value)
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)
  if ((url.protocol !== "https:" && !(url.protocol === "http:" && local)) || url.username || url.password || url.search || url.hash || url.pathname !== "/") {
    throw new AuthError(503, "auth_not_configured", "Authentication service URLs must be HTTPS origins.")
  }
  return url
}

export function adminOrigin(): string {
  return configuredUrl(process.env.OMNI_ADMIN_ORIGIN || "https://admin.omni.teamofsilicons.com").origin
}

function issuer(): string {
  return configuredUrl(process.env.SILICON_ACCOUNTS_URL || "https://accounts.teamofsilicons.com").origin
}

function appSecret(): string {
  const secret = process.env.SILICON_APP_SECRET
  if (!secret || secret.length < 16) throw new AuthError(503, "auth_not_configured", "Silicon Accounts sign-in is not configured.")
  return secret
}

export function authJson(value: unknown, status = 200): Response {
  return Response.json(value, { status, headers: { "Cache-Control": "no-store", "Pragma": "no-cache" } })
}

export function authError(error: unknown): Response {
  const known = error instanceof AuthError
  return authJson({ error: { code: known ? error.code : "authentication_failed", message: known ? error.message : "Sign-in could not be completed. Please try again." } }, known ? error.status : 500)
}

export async function readAuthJson(request: Request): Promise<Json> {
  if (!request.headers.get("content-type")?.startsWith("application/json")) throw new AuthError(415, "invalid_request", "Send an application/json request.")
  if (Number(request.headers.get("content-length")) > 16384) throw new AuthError(413, "invalid_request", "The request is too large.")
  const text = await request.text()
  if (text.length > 16384) throw new AuthError(413, "invalid_request", "The request is too large.")
  let body: unknown
  try { body = JSON.parse(text) } catch { throw new AuthError(400, "invalid_request", "Send a JSON object.") }
  if (!body || typeof body !== "object" || Array.isArray(body)) throw new AuthError(400, "invalid_request", "Send a JSON object.")
  return body as Json
}

function credential(value: unknown, name: string): string {
  if (typeof value !== "string" || !value || value.length > 8192 || /\s/.test(value)) throw new AuthError(400, "invalid_request", `A valid ${name} is required.`)
  return value
}

async function accounts(path: string, body: Json): Promise<Json> {
  let response: Response
  try {
    response = await fetch(`${issuer()}${path}`, {
      method: "POST", redirect: "error", cache: "no-store", signal: AbortSignal.timeout(15000),
      headers: { "Content-Type": "application/json", "Authorization": `Basic ${Buffer.from(`${APP_ID}:${appSecret()}`).toString("base64")}` },
      body: JSON.stringify(body),
    })
  } catch (error) {
    if (error instanceof AuthError) throw error
    throw new AuthError(503, "accounts_unavailable", "Silicon Accounts is unavailable. Try again shortly.")
  }
  let data: Json
  try { data = await response.json() } catch { throw new AuthError(503, "accounts_unavailable", "Silicon Accounts returned an invalid response.") }
  if (!data || typeof data !== "object" || Array.isArray(data)) throw new AuthError(503, "accounts_unavailable", "Silicon Accounts returned an invalid response.")
  if (!response.ok) {
    const code = typeof data.error === "string" ? data.error : (data.error as Json | undefined)?.code
    const messages: Record<string, string> = {
      authorization_pending: "Approve sign-in in Silicon Accounts.", slow_down: "Wait before checking sign-in again.",
      expired_token: "This sign-in request expired. Begin a new sign-in.", access_denied: "Sign-in was declined.",
      invalid_grant: "This sign-in request expired or was already used. Begin a new sign-in.",
    }
    if (typeof code === "string" && messages[code]) throw new AuthError(code === "authorization_pending" || code === "slow_down" ? 400 : 401, code, messages[code])
    throw new AuthError(503, "accounts_unavailable", "Silicon Accounts could not complete this request.")
  }
  return data
}

function seal(value: Json, purpose: string): string {
  const iv = randomBytes(12)
  const key = createHmac("sha256", appSecret()).update(`omni:${purpose}:v1`).digest()
  const cipher = createCipheriv("aes-256-gcm", key, iv)
  cipher.setAAD(Buffer.from(purpose))
  const encrypted = Buffer.concat([cipher.update(JSON.stringify(value), "utf8"), cipher.final()])
  return Buffer.concat([iv, cipher.getAuthTag(), encrypted]).toString("base64url")
}

function unseal(value: string, purpose: string): Json {
  const key = createHmac("sha256", appSecret()).update(`omni:${purpose}:v1`).digest()
  try {
    if (!/^[A-Za-z0-9_-]+$/.test(value) || value.length > 12000) throw new Error()
    const bytes = Buffer.from(value, "base64url")
    const decipher = createDecipheriv("aes-256-gcm", key, bytes.subarray(0, 12))
    decipher.setAAD(Buffer.from(purpose))
    decipher.setAuthTag(bytes.subarray(12, 28))
    const data = JSON.parse(Buffer.concat([decipher.update(bytes.subarray(28)), decipher.final()]).toString("utf8"))
    if (!data || typeof data !== "object" || !Number.isInteger(data.exp) || data.exp <= Date.now() / 1000) throw new Error()
    return data
  } catch { throw new AuthError(401, "session_expired", "Your session expired. Sign in again.") }
}

function cookieName(kind: "session" | "login"): string {
  return `${adminOrigin().startsWith("https:") ? "__Host-" : ""}omni_${kind}`
}

function cookieValue(request: Request, kind: "session" | "login"): string | undefined {
  const name = cookieName(kind)
  const values = (request.headers.get("cookie") || "").split(";").map(x => x.trim()).filter(x => x.startsWith(`${name}=`))
  if (values.length > 1) throw new AuthError(400, "invalid_request", "Ambiguous session cookie.")
  return values[0]?.slice(name.length + 1)
}

function setCookie(response: Response, kind: "session" | "login", value: string, age: number): void {
  response.headers.append("Set-Cookie", `${cookieName(kind)}=${value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${age}${adminOrigin().startsWith("https:") ? "; Secure" : ""}`)
}

export function checkAuthOrigin(request: Request): void {
  const origin = request.headers.get("origin")
  if (origin && (origin !== adminOrigin() || new URL(request.url).origin !== adminOrigin())) throw new AuthError(403, "invalid_origin", "Sign-in requests must come from the Omni admin site.")
}

export function safeReturnPath(value: string | null): string {
  const path = value || "/admin"
  if (!path.startsWith("/") || path.startsWith("//") || /[\\\u0000-\u001f\u007f]/.test(path)) throw new AuthError(400, "invalid_return", "The return path must stay within Omni admin.")
  const url = new URL(path, adminOrigin())
  if (url.origin !== adminOrigin()) throw new AuthError(400, "invalid_return", "The return path must stay within Omni admin.")
  return url.pathname + url.search + url.hash
}

export interface Account { uuid: string; id: string; kind: "carbon" | "silicon"; display_name: string }
export interface AuthSession { authenticated: true; account: Account; is_admin: boolean; expires_at: string }

function isAdmin(uuid: string): boolean {
  return (process.env.OMNI_ADMIN_IDS || "").split(",").map(x => x.trim()).filter(x => UUID.test(x)).includes(uuid)
}

async function verifyAccess(accessToken: string, uuid?: string): Promise<{ account: Account; exp: number }> {
  const seen = await accounts("/v1/oauth/introspect", { token: accessToken })
  if (seen.active !== true) throw new AuthError(401, "session_expired", "Your session expired or was revoked. Sign in again.")
  if (seen.aud !== APP_ID || seen.client_id !== APP_ID || seen.token_type !== "access_token" ||
      typeof seen.sub !== "string" || !UUID.test(seen.sub) || (uuid && seen.sub !== uuid) ||
      (seen.kind !== "carbon" && seen.kind !== "silicon") || typeof seen.id !== "string" ||
      !seen.id.startsWith(seen.kind === "carbon" ? "c:" : "si:") || typeof seen.exp !== "number" || seen.exp <= Date.now() / 1000 ||
      (seen.iss !== undefined && seen.iss !== issuer())) {
    throw new AuthError(401, "invalid_session", "Silicon Accounts returned a mismatched identity.")
  }
  return { account: { uuid: seen.sub, id: seen.id, kind: seen.kind, display_name: typeof seen.display_name === "string" ? seen.display_name : seen.id }, exp: seen.exp }
}

function sessionInfo(account: Account, exp: number): AuthSession {
  return { authenticated: true, account, is_admin: isAdmin(account.uuid), expires_at: new Date(exp * 1000).toISOString() }
}

function requestToken(request: Request): string | undefined {
  const authorization = request.headers.get("authorization")
  if (authorization) {
    if (!/^Bearer [A-Za-z0-9_-]+$/.test(authorization)) throw new AuthError(401, "invalid_session", "A valid Omni bearer session is required.")
    return authorization.slice(7)
  }
  if (new URL(request.url).origin !== adminOrigin()) return undefined
  return cookieValue(request, "session")
}

async function loadSession(request: Request): Promise<{ info: AuthSession; accessToken: string } | null> {
  const token = requestToken(request)
  if (!token) return null
  const saved = unseal(token, "session")
  if (typeof saved.access_token !== "string" || typeof saved.uuid !== "string") throw new AuthError(401, "invalid_session", "Sign in again.")
  const verified = await verifyAccess(saved.access_token, saved.uuid)
  return { info: sessionInfo(verified.account, Math.min(saved.exp as number, verified.exp)), accessToken: saved.access_token }
}

export async function getSession(request: Request): Promise<AuthSession | { authenticated: false }> {
  try { return (await loadSession(request))?.info || { authenticated: false } }
  catch (error) { if (error instanceof AuthError && error.status === 401) return { authenticated: false }; throw error }
}

export async function requireAdmin(request: Request): Promise<AuthSession> {
  if (!["GET", "HEAD", "OPTIONS"].includes(request.method)) {
    checkAuthOrigin(request)
    if (!request.headers.has("authorization") && request.headers.get("origin") !== adminOrigin()) throw new AuthError(403, "invalid_origin", "Admin changes require the Omni admin origin.")
  }
  const session = await loadSession(request)
  if (!session) throw new AuthError(401, "sign_in_required", "Sign in with Silicon Accounts to continue.")
  if (!session.info.is_admin) throw new AuthError(403, "admin_required", "This account is not an Omni administrator.")
  return session.info
}

async function exchangeSession(grant: Json): Promise<{ token: string; info: AuthSession }> {
  const pair = await accounts("/v1/oauth/token", grant)
  if (pair.token_type !== "Bearer" || typeof pair.access_token !== "string" || typeof pair.expires_in !== "number" || pair.expires_in <= 0) throw new AuthError(503, "accounts_unavailable", "Silicon Accounts returned an invalid session.")
  const account = pair.account as Json | undefined
  if (typeof account?.uuid !== "string") throw new AuthError(503, "accounts_unavailable", "Silicon Accounts omitted the account identity.")
  const verified = await verifyAccess(pair.access_token, account.uuid)
  const exp = Math.floor(Math.min(verified.exp, Date.now() / 1000 + pair.expires_in, Date.now() / 1000 + SESSION_SECONDS))
  // shortcut: sessions end with the access token; add durable serialized refresh storage before offering persistent login.
  const token = seal({ access_token: pair.access_token, uuid: verified.account.uuid, exp }, "session")
  if (token.length > 3800) throw new AuthError(503, "accounts_unavailable", "The Accounts session exceeds the supported size.")
  return { token, info: sessionInfo(verified.account, exp) }
}

export async function loginWithGrant(request: Request, grant: "slt" | "device"): Promise<Response> {
  checkAuthOrigin(request)
  const body = await readAuthJson(request)
  const value = grant === "slt" ? { grant_type: "urn:silicon:params:oauth:grant-type:slt", slt: credential(body.slt, "short-lived token") } : { grant_type: "urn:ietf:params:oauth:grant-type:device_code", device_code: credential(body.device_code, "device code") }
  const session = await exchangeSession(value)
  const browser = request.headers.has("origin")
  const response = authJson(browser ? session.info : { ...session.info, session_token: session.token })
  if (browser) setCookie(response, "session", session.token, Math.max(0, Math.floor((Date.parse(session.info.expires_at) - Date.now()) / 1000)))
  return response
}

export async function startDevice(request: Request): Promise<Response> {
  checkAuthOrigin(request)
  const result = await accounts("/v1/device/authorize", { client_id: APP_ID, client_label: "Omni CLI" })
  const fields = ["device_code", "user_code", "verification_uri", "verification_uri_complete", "expires_in", "interval"]
  if (typeof result.device_code !== "string" || typeof result.user_code !== "string" || typeof result.verification_uri !== "string" || typeof result.expires_in !== "number") throw new AuthError(503, "accounts_unavailable", "Silicon Accounts returned an invalid device sign-in.")
  for (const field of ["verification_uri", "verification_uri_complete"]) {
    if (result[field] !== undefined && (typeof result[field] !== "string" || new URL(result[field] as string).origin !== issuer())) throw new AuthError(503, "accounts_unavailable", "Silicon Accounts returned an invalid sign-in destination.")
  }
  return authJson(Object.fromEntries(fields.filter(field => result[field] !== undefined).map(field => [field, result[field]])))
}

export function startBrowser(request: Request): Response {
  if (new URL(request.url).origin !== adminOrigin()) throw new AuthError(400, "invalid_origin", "Open sign-in on the Omni admin site.")
  const state = randomBytes(32).toString("base64url"), verifier = randomBytes(32).toString("base64url")
  const returnTo = safeReturnPath(new URL(request.url).searchParams.get("return_to"))
  const url = new URL("/authorize", issuer())
  url.search = new URLSearchParams({ app_id: APP_ID, response_type: "code", redirect_uri: `${adminOrigin()}/auth/callback`, state, code_challenge: createHash("sha256").update(verifier).digest("base64url"), code_challenge_method: "S256", scope: "profile" }).toString()
  const response = new Response(null, { status: 302, headers: { Location: url.toString(), "Cache-Control": "no-store", "Referrer-Policy": "no-referrer" } })
  setCookie(response, "login", seal({ state, verifier, returnTo, exp: Math.floor(Date.now() / 1000) + 600 }, "login"), 600)
  return response
}

export async function completeBrowser(request: Request): Promise<Response> {
  if (new URL(request.url).origin !== adminOrigin()) throw new AuthError(400, "invalid_origin", "Open sign-in on the Omni admin site.")
  try {
    const query = new URL(request.url).searchParams
    const savedCookie = cookieValue(request, "login")
    if (!savedCookie) throw new AuthError(401, "invalid_state", "Begin sign-in again from Omni admin.")
    const saved = unseal(savedCookie, "login"), state = query.get("state") || ""
    if (typeof saved.state !== "string" || state.length !== saved.state.length || !timingSafeEqual(Buffer.from(state), Buffer.from(saved.state))) throw new AuthError(401, "invalid_state", "The sign-in request did not match. Begin sign-in again.")
    if (query.has("error")) throw new AuthError(401, "access_denied", "Sign-in was not completed.")
    const session = await exchangeSession({ grant_type: "authorization_code", code: credential(query.get("code"), "authorization code"), code_verifier: saved.verifier, redirect_uri: `${adminOrigin()}/auth/callback` })
    const response = new Response(null, { status: 303, headers: { Location: `${adminOrigin()}${safeReturnPath(typeof saved.returnTo === "string" ? saved.returnTo : null)}`, "Cache-Control": "no-store", "Referrer-Policy": "no-referrer" } })
    setCookie(response, "session", session.token, Math.max(0, Math.floor((Date.parse(session.info.expires_at) - Date.now()) / 1000)))
    setCookie(response, "login", "", 0)
    return response
  } catch (error) {
    const response = authError(error)
    setCookie(response, "login", "", 0)
    return response
  }
}

export async function logout(request: Request): Promise<Response> {
  checkAuthOrigin(request)
  if (!request.headers.has("authorization") && request.headers.get("origin") !== adminOrigin()) throw new AuthError(403, "invalid_origin", "Sign out from the Omni admin site.")
  let session: Awaited<ReturnType<typeof loadSession>> = null
  try { session = await loadSession(request) } catch (error) { if (!(error instanceof AuthError && error.status === 401)) throw error }
  if (session) await accounts("/v1/oauth/revoke", { token: session.accessToken })
  const response = authJson({ authenticated: false })
  if (new URL(request.url).origin === adminOrigin()) setCookie(response, "session", "", 0)
  return response
}
