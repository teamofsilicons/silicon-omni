"use client"

import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react"
import { ArrowDown, ArrowRight, ArrowUp, Check, DownloadSimple, MagnifyingGlass, Plus, SignOut, X } from "@phosphor-icons/react/dist/ssr"
import { CATEGORIES, modelIdentity, upgradeCatalog, validateCatalog, type Catalog, type CatalogEntry } from "@/lib/catalog"
import { PROVIDERS, canRunFast } from "@/lib/providers"
import type { Entry, Key } from "@/lib/choose"
import "./admin.css"

type Session = { authenticated: false } | {
  authenticated: true
  account: { uuid: string; id: string; display_name: string }
  is_admin: boolean
  expires_at: string
}
type Snapshot = { catalog: Catalog; revision: string; writable: boolean }
type ModelForm = { provider: string; model: string; effort: string; status: string; notes: string; scores: string; price: string; secs: string; sources: string }
type Editor = { identity: string | null; original: CatalogEntry | null; initial: ModelForm; form: ModelForm }
type SavedDraft = { account: string; base: Catalog; catalog: Catalog; revision: string }
const LOGIN = "/auth/login?return_to=/admin"
const DRAFT_KEY = "omni.admin.draft.v1"
const LABELS: Record<string, string> = { "codex-app-server": "Codex", "claude-code-cli": "Claude Code", "antigravity-cli": "Antigravity" }

class ApiError extends Error {
  constructor(public status: number, message: string) { super(message) }
}
async function request<T>(url: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(url, { cache: "no-store", credentials: "same-origin", ...options })
  const body = await response.json().catch(() => null)
  if (!response.ok) {
    const message = typeof body?.error === "string" ? body.error : body?.error?.message
    throw new ApiError(response.status, typeof message === "string" ? message : `Request failed (${response.status}). Please try again.`)
  }
  if (body === null) throw new Error("The server returned an unreadable response. Please try again.")
  return body as T
}
function message(error: unknown): string { return error instanceof Error ? error.message : "Something went wrong. Please try again." }
function same(a: unknown, b: unknown): boolean { return JSON.stringify(a) === JSON.stringify(b) }
function status(entry: CatalogEntry): string { return entry.status ?? "current" }
function title(entry: Entry): string { return `${entry.model}${entry.effort ? ` · ${entry.effort}` : ""}` }
function formOf(entry?: CatalogEntry): ModelForm {
  return {
    provider: entry?.provider ?? "codex-app-server", model: entry?.model ?? "", effort: entry ? entry.effort ?? "" : "medium",
    status: entry?.status ?? "current", notes: entry?.notes ?? "", scores: JSON.stringify(entry?.scores ?? null, null, 2),
    price: entry?.price == null ? "" : String(entry.price), secs: entry?.secs == null ? "" : String(entry.secs),
    sources: Array.isArray(entry?.sources) ? entry.sources.filter(s => typeof s === "string").join("\n") : "",
  }
}
function readDraft(account: string): SavedDraft | null {
  try {
    const saved = JSON.parse(sessionStorage.getItem(DRAFT_KEY) || "null") as SavedDraft | null
    if (!saved || saved.account !== account || typeof saved.revision !== "string") return null
    saved.base = upgradeCatalog(saved.base)
    // A description can be temporarily empty while editing; retain the rest of that draft.
    const upgraded = upgradeCatalog({ ...saved.catalog, keys: Object.fromEntries(Object.entries(saved.catalog.keys).map(([name, key]) =>
      [name, { ...key, says: typeof key.says === "string" && !key.says.trim() ? "Draft description" : key.says }],
    )) })
    saved.catalog = { ...upgraded, keys: { ...upgraded.keys, ...saved.catalog.keys } }
    return saved
  } catch { return null }
}
function removeDraft() { try { sessionStorage.removeItem(DRAFT_KEY) } catch { /* Browser storage may be disabled. */ } }
function downloadDraft(catalog: Catalog, revision: string) {
  const blob = new Blob([JSON.stringify({ catalog, revision }, null, 2) + "\n"], { type: "application/json" })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.download = `omni-catalog-draft-${new Date().toISOString().slice(0, 10)}.json`
  anchor.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export default function AdminPage() {
  const [session, setSession] = useState<Session | null>(null)
  const [base, setBase] = useState<Catalog | null>(null)
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [revision, setRevision] = useState("")
  const [writable, setWritable] = useState(false)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState<"publish" | "reload" | "logout" | null>(null)
  const [error, setError] = useState("")
  const [notice, setNotice] = useState("")
  const [conflict, setConflict] = useState(false)
  const [expired, setExpired] = useState(false)
  const [storageWarning, setStorageWarning] = useState(false)
  const [view, setView] = useState<"models" | "categories">("models")
  const [query, setQuery] = useState("")
  const [providerFilter, setProviderFilter] = useState("")
  const [statusFilter, setStatusFilter] = useState("")
  const [editor, setEditor] = useState<Editor | null>(null)
  const [editorError, setEditorError] = useState("")
  const [category, setCategory] = useState<string>("general")
  const [addPick, setAddPick] = useState("")
  const [categoryError, setCategoryError] = useState("")
  const modelInput = useRef<HTMLInputElement>(null)
  const dirty = Boolean(base && catalog && !same(base, catalog))
  const editorDirty = Boolean(editor && !same(editor.initial, editor.form))
  const disabled = !writable || Boolean(busy) || expired

  const load = useCallback(async (signal?: AbortSignal) => {
    setLoading(true)
    setError("")
    try {
      const auth = await request<Session>("/api/auth/session", { signal })
      setSession(auth)
      if (auth.authenticated && auth.is_admin) {
        const data = await request<Snapshot>("/api/admin/catalog", { signal })
        validateCatalog(data.catalog)
        const saved = readDraft(auth.account.uuid)
        setWritable(data.writable)
        setBase(saved?.base ?? data.catalog)
        setCatalog(saved?.catalog ?? data.catalog)
        setRevision(saved?.revision ?? data.revision)
        setConflict(Boolean(saved && saved.revision !== data.revision))
        if (saved) setNotice("Your staged draft has been restored in this tab.")
      }
    } catch (error) {
      if (signal?.aborted) return
      setError(message(error))
    } finally { if (!signal?.aborted) setLoading(false) }
  }, [])

  useEffect(() => {
    const controller = new AbortController()
    void load(controller.signal)
    return () => controller.abort()
  }, [load])
  useEffect(() => {
    if (!session?.authenticated || !catalog || !base) return
    try {
      if (dirty) sessionStorage.setItem(DRAFT_KEY, JSON.stringify({ account: session.account.uuid, base, catalog, revision } satisfies SavedDraft))
      else sessionStorage.removeItem(DRAFT_KEY)
      setStorageWarning(false)
    } catch { setStorageWarning(dirty) }
  }, [session, base, catalog, revision, dirty])
  useEffect(() => {
    if (!dirty && !editorDirty) return
    const warn = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = "" }
    window.addEventListener("beforeunload", warn)
    return () => window.removeEventListener("beforeunload", warn)
  }, [dirty, editorDirty])
  useEffect(() => { if (editor) modelInput.current?.focus() }, [editor?.identity])

  const changes = useMemo(() => {
    if (!catalog || !base) return []
    const previous = new Map(base.models.map(m => [modelIdentity(m), m]))
    const current = new Set(catalog.models.map(modelIdentity))
    const result: string[] = []
    for (const model of catalog.models) {
      const before = previous.get(modelIdentity(model))
      if (!before) result.push(`Add ${title(model)} (${LABELS[model.provider]})`)
      else if (!same(before, model)) result.push(`Update ${title(model)} (${LABELS[model.provider]})`)
    }
    for (const model of base.models) if (!current.has(modelIdentity(model))) result.push(`Replace ${title(model)} (${LABELS[model.provider]})`)
    for (const name of CATEGORIES) if (!same(base.keys[name], catalog.keys[name])) result.push(`Update ${name} recommendations`)
    return result
  }, [base, catalog])
  const visible = useMemo(() => (catalog?.models ?? []).filter(m =>
    (!providerFilter || m.provider === providerFilter) && (!statusFilter || status(m) === statusFilter) &&
    `${m.model} ${m.effort ?? ""} ${m.notes ?? ""} ${LABELS[m.provider]}`.toLowerCase().includes(query.trim().toLowerCase()),
  ), [catalog, providerFilter, statusFilter, query])
  const currentRows = useMemo(() => (catalog?.models ?? []).filter(m => status(m) === "current"), [catalog])

  function handleFailure(error: unknown) {
    setError(message(error))
    if (error instanceof ApiError && error.status === 401) setExpired(true)
    if (error instanceof ApiError && error.status === 403) setWritable(false)
  }
  function openEditor(entry?: CatalogEntry) {
    if (editorDirty && !window.confirm("Discard the unstaged edits in the model editor?")) return
    const form = formOf(entry)
    setEditor({ identity: entry ? modelIdentity(entry) : null, original: entry ?? null, initial: form, form })
    setEditorError("")
    setView("models")
  }
  function closeEditor() {
    if (editorDirty && !window.confirm("Discard the unstaged edits in the model editor?")) return
    setEditor(null)
    setEditorError("")
  }
  function setField(field: keyof ModelForm, value: string) {
    setEditor(e => {
      if (!e) return e
      const form = { ...e.form, [field]: value }
      if (field === "provider" && !PROVIDERS.find(p => p.id === value)?.efforts.includes(form.effort)) form.effort = ""
      return { ...e, form }
    })
    setEditorError("")
  }
  function stageModel(event: FormEvent) {
    event.preventDefault()
    if (!catalog || !editor || disabled) return
    setEditorError("")
    try {
      const form = editor.form
      const measured = (value: string) => value.trim() === "" ? null : Number(value)
      let scores: unknown
      try { scores = JSON.parse(form.scores.trim() || "null") } catch { throw new Error("Benchmark scores must be valid JSON: an object of numbers, or null.") }
      const entry: CatalogEntry = {
        ...editor.original, provider: form.provider, model: form.model.trim(), effort: form.effort,
        status: form.status as CatalogEntry["status"], notes: form.notes.trim(),
        scores: scores as CatalogEntry["scores"], price: measured(form.price), secs: measured(form.secs),
      }
      // Only replace sources when edited; unknown metadata stays on the original row.
      if (form.sources !== editor.initial.sources || !editor.original) entry.sources = form.sources.split("\n").map(s => s.trim()).filter(Boolean)
      const id = modelIdentity(entry)
      const usedBy = CATEGORIES.filter(name => catalog.keys[name].picks.some(p => modelIdentity(p) === editor.identity))
      if (status(entry) !== "current" && usedBy.length) throw new Error(`Remove this model from ${usedBy.join(", ")} recommendations before changing its status.`)
      const models = editor.identity === null ? [...catalog.models, entry] : catalog.models.map(m => modelIdentity(m) === editor.identity ? entry : m)
      const keys = Object.fromEntries(Object.entries(catalog.keys).map(([name, key]) => [name, {
        ...key, picks: key.picks.map(p => id !== editor.identity && modelIdentity(p) === editor.identity ? { ...p, provider: entry.provider, model: entry.model, effort: entry.effort ?? "", fast: Boolean(p.fast) && canRunFast(entry.provider, entry.model) } : p),
      }]))
      const next = validateCatalog({ ...catalog, models, keys })
      setCatalog(next)
      const saved = formOf(entry)
      setEditor({ identity: id, original: entry, initial: saved, form: saved })
      setNotice(`${title(entry)} is staged. Publish changes when you are ready.`)
      setError("")
    } catch (error) { setEditorError(message(error)) }
  }
  function updateCategory(next: Key) {
    if (!catalog || disabled) return
    try {
      setCatalog(validateCatalog({ ...catalog, keys: { ...catalog.keys, [category]: next } }))
      setCategoryError("")
      setNotice("")
    } catch (error) { setCategoryError(message(error)) }
  }
  function movePick(index: number, direction: -1 | 1) {
    if (!catalog) return
    const key = catalog.keys[category]
    const picks = [...key.picks]
    ;[picks[index], picks[index + direction]] = [picks[index + direction], picks[index]]
    updateCategory({ ...key, picks })
  }
  function appendPick() {
    if (!catalog) return
    const found = currentRows.find(m => modelIdentity(m) === addPick)
    if (!found) return
    const key = catalog.keys[category]
    updateCategory({ ...key, picks: [...key.picks, { provider: found.provider, model: found.model, effort: found.effort ?? "", fast: false, requires_availability: Boolean(found.requires_availability) }] })
    setAddPick("")
  }
  async function publish() {
    if (!catalog || disabled || !dirty || editorDirty || conflict) return
    setBusy("publish")
    setError("")
    setNotice("")
    try {
      validateCatalog(catalog)
      const data = await request<Snapshot>("/api/admin/catalog", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ catalog, revision }) })
      validateCatalog(data.catalog)
      setBase(data.catalog)
      setCatalog(data.catalog)
      setRevision(data.revision)
      setWritable(data.writable)
      removeDraft()
      setNotice("Changes published. The new catalog and recommendations are live.")
      setEditor(null)
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) {
        setConflict(true)
        setError("")
      } else handleFailure(error)
    } finally { setBusy(null) }
  }
  async function reloadLatest() {
    if (editorDirty && !window.confirm("The unstaged model edits will be discarded. Continue?")) return
    setBusy("reload")
    setError("")
    try {
      const data = await request<Snapshot>("/api/admin/catalog")
      validateCatalog(data.catalog)
      if (dirty && catalog) downloadDraft(catalog, revision)
      setBase(data.catalog)
      setCatalog(data.catalog)
      setRevision(data.revision)
      setWritable(data.writable)
      setConflict(false)
      setExpired(false)
      setEditor(null)
      removeDraft()
      setNotice(dirty ? "Your draft was downloaded. The latest catalog is loaded; reapply the changes you want to publish." : "The latest catalog is loaded.")
    } catch (error) { handleFailure(error) }
    finally { setBusy(null) }
  }
  function discard() {
    if (!base || !window.confirm("Discard all unpublished changes and model editor edits?")) return
    setCatalog(base)
    setEditor(null)
    removeDraft()
    setNotice("Unpublished changes discarded.")
    setError("")
  }
  async function logout() {
    if (editorDirty && !window.confirm("Sign out and discard the unstaged model edits? Staged changes will stay in this tab.")) return
    setBusy("logout")
    setError("")
    try {
      await request<Session>("/api/auth/logout", { method: "POST" })
      // Keep staged data in session storage, but remove it from the signed-out view.
      setSession({ authenticated: false })
      setCatalog(null)
      setBase(null)
      setEditor(null)
      setNotice("")
    } catch (error) { handleFailure(error) }
    finally { setBusy(null) }
  }

  const activeKey = catalog?.keys[category]
  return (
    <div className="omni-admin">
      <header className="admin-heading">
        <div>
          <p className="admin-eyebrow">silicon omni / administration</p>
          <h1>Keep the right<br />models in reach.</h1>
          <p className="admin-intro">One catalog. Six categories. A better default as models evolve.</p>
        </div>
        {session?.authenticated && <div className="admin-account">
          <span className="admin-account-dot" aria-hidden />
          <div><strong>{session.account.display_name || session.account.id}</strong><span>{session.account.id}</span></div>
          <button type="button" className="admin-icon-button" aria-label="Sign out" onClick={() => void logout()} disabled={Boolean(busy)}><SignOut size={18} /></button>
        </div>}
      </header>

      {error && <div className="admin-message admin-error" role="alert"><strong>Could not complete the request.</strong><span>{error}</span>{expired && <a className="admin-text-link" href={LOGIN}>Sign in again <ArrowRight size={14} /></a>}</div>}
      {loading ? <div className="admin-state" role="status"><span className="admin-loading" aria-hidden />Checking your access…</div> : !session?.authenticated ? (
        <section className="admin-welcome" aria-labelledby="admin-sign-in-title">
          <div className="admin-welcome-copy"><span className="admin-eyebrow">A single place to manage the lineup</span><h2 id="admin-sign-in-title">Sign in with Silicon Accounts.</h2><p>Add new releases, keep older models available, and decide what Omni recommends for each kind of work.</p><a className="admin-button admin-primary" href={LOGIN}>Continue with Silicon Accounts <ArrowRight size={17} /></a>{error && <button className="admin-text-link" type="button" onClick={() => void load()}>Try checking access again</button>}</div>
          <div className="admin-welcome-outline"><span className="admin-eyebrow">The seven defaults you control</span>{CATEGORIES.map((name, index) => <div key={name}><span className="admin-order">0{index + 1}</span><span>{name}</span><ArrowRight size={14} /></div>)}<p>Changes are staged before you publish.</p></div>
        </section>
      ) : !session.is_admin ? (
        <section className="admin-state admin-access"><span className="admin-eyebrow">Signed in successfully</span><h2>This account needs admin access.</h2><p><strong>{session.account.id}</strong> can sign in, but cannot change the Omni catalog. An Omni administrator must grant access to this account.</p><div className="admin-actions"><a className="admin-button" href="/models">View the public catalog</a><button className="admin-button" type="button" onClick={() => void logout()} disabled={Boolean(busy)}>Use another account</button></div></section>
      ) : !catalog ? (
        <section className="admin-state"><h2>The catalog could not be loaded.</h2><p>Your account is signed in. Try loading the catalog again.</p><button className="admin-button" onClick={() => void load()} type="button">Retry</button></section>
      ) : <>
        <div className="admin-publish-bar">
          <div><span className={`admin-save-dot${dirty ? " changed" : ""}`} aria-hidden /><strong>{dirty ? `${changes.length} staged ${changes.length === 1 ? "change" : "changes"}` : "Catalog is up to date"}</strong><small>{editorDirty ? "Stage or cancel the model edit before publishing." : dirty ? "Review your draft, then publish it for everyone." : "Changes you make here stay in draft until published."}</small></div>
          <div className="admin-actions"><button className="admin-button" type="button" onClick={discard} disabled={Boolean(busy) || (!dirty && !editorDirty)}>Discard</button><button className="admin-button admin-primary" type="button" onClick={() => void publish()} disabled={disabled || !dirty || editorDirty || conflict}>{busy === "publish" ? "Publishing…" : "Publish changes"}<ArrowRight size={16} /></button></div>
        </div>
        {notice && <p className="admin-notice" role="status"><Check size={16} aria-hidden />{notice}</p>}
        {!writable && <div className="admin-message" role="status"><strong>This catalog is read-only.</strong><span>You can inspect the catalog, but publishing is unavailable on this deployment.</span></div>}
        {storageWarning && <div className="admin-message" role="alert"><strong>Your browser cannot save this draft in the tab.</strong><span>Keep this page open, or download the draft before leaving.</span><button className="admin-button" type="button" onClick={() => downloadDraft(catalog, revision)}><DownloadSimple size={16} />Download draft</button></div>}
        {conflict && <section className="admin-message admin-conflict" role="alert"><div><strong>A newer catalog has been published.</strong><p>Your staged draft is preserved. Download it and load the latest catalog before reapplying your changes. Publishing is paused so another person's work is not overwritten.</p></div><button className="admin-button" type="button" onClick={() => downloadDraft(catalog, revision)}><DownloadSimple size={16} />Download draft</button><button className="admin-button admin-primary" type="button" onClick={() => void reloadLatest()} disabled={Boolean(busy)}>{busy === "reload" ? "Loading…" : "Download draft & load latest"}</button></section>}
        {dirty && <details className="admin-review"><summary>Review staged changes <span>{changes.length}</span></summary><ul>{changes.map((change, index) => <li key={index}>{change}</li>)}</ul></details>}
        <div className="admin-summary"><div><span className="admin-stat">{catalog.models.length}</span><span>model configurations</span></div><div><span className="admin-stat">{currentRows.length}</span><span>current configurations</span></div><div><span className="admin-stat">{CATEGORIES.length}</span><span>curated categories</span></div><p>Keep older models in the catalog.<br />Recommend the ones you would use today.</p></div>
        <div className="admin-section-nav" role="group" aria-label="Catalog editor sections"><button type="button" aria-pressed={view === "models"} onClick={() => setView("models")}>Models <span>{catalog.models.length}</span></button><button type="button" aria-pressed={view === "categories"} onClick={() => setView("categories")}>Category recommendations <span>6</span></button><a href="/models" target="_blank" rel="noreferrer">View public catalog <ArrowRight size={14} /></a></div>
        {view === "models" ? <div className={`admin-catalog-layout${editor ? " editing" : ""}`}>
          <section className="admin-models" aria-label="Model configurations">
            <div className="admin-filters"><label className="admin-search"><span className="said">Search models</span><MagnifyingGlass size={17} aria-hidden /><input value={query} onChange={e => setQuery(e.target.value)} placeholder="Search models or notes" type="search" /></label><label><span className="said">Filter by provider</span><select value={providerFilter} onChange={e => setProviderFilter(e.target.value)}><option value="">All providers</option>{PROVIDERS.map(p => <option key={p.id} value={p.id}>{LABELS[p.id]}</option>)}</select></label><label><span className="said">Filter by status</span><select value={statusFilter} onChange={e => setStatusFilter(e.target.value)}><option value="">All statuses</option><option value="current">Current</option><option value="legacy">Legacy</option><option value="restricted">Restricted</option></select></label><button className="admin-button admin-add" type="button" disabled={disabled} onClick={() => openEditor()}><Plus size={16} />Add model</button></div>
            <p className="admin-result-count" aria-live="polite">{visible.length} of {catalog.models.length} configurations</p>
            <div className="admin-table-wrap"><table className="admin-model-table"><thead><tr><th scope="col">Model</th><th scope="col">Effort</th><th scope="col">Status</th><th scope="col"><span className="said">Actions</span></th></tr></thead><tbody>{visible.map(m => <tr key={modelIdentity(m)} className={editor?.identity === modelIdentity(m) ? "selected" : ""}><th scope="row"><span className="admin-model-name">{m.model}</span><span className="admin-model-provider">{LABELS[m.provider]}</span></th><td>{m.effort || "In model ID"}</td><td><span className={`admin-status ${status(m)}`}>{status(m)}</span></td><td><button type="button" className="admin-edit" onClick={() => openEditor(m)} disabled={Boolean(busy)} aria-label={`${writable ? "Edit" : "View"} ${title(m)}, ${LABELS[m.provider]}`}>{writable ? "Edit" : "View"}<ArrowRight size={13} /></button></td></tr>)}</tbody></table>{visible.length === 0 && <div className="admin-empty"><h3>No models match your filters.</h3><p>Try a different model name, provider, or status.</p><button className="admin-text-link" type="button" onClick={() => { setQuery(""); setProviderFilter(""); setStatusFilter("") }}>Clear filters</button></div>}</div>
          </section>
          {editor && <aside className="admin-editor" aria-labelledby="model-editor-title"><div className="admin-panel-heading"><div><span className="admin-eyebrow">Model configuration</span><h2 id="model-editor-title">{editor.identity === null ? "Add a model" : "Edit model"}</h2></div><button className="admin-icon-button" type="button" aria-label="Close model editor" onClick={closeEditor}><X size={18} /></button></div><form onSubmit={stageModel}><fieldset disabled={disabled}><label>Model ID<input ref={modelInput} value={editor.form.model} onChange={e => setField("model", e.target.value)} required maxLength={120} placeholder="gpt-6.1-sol" autoComplete="off" spellCheck={false} /><small>The exact ID accepted by the provider.</small></label><div className="admin-form-pair"><label>Provider<select value={editor.form.provider} onChange={e => setField("provider", e.target.value)}>{PROVIDERS.map(p => <option key={p.id} value={p.id}>{LABELS[p.id]}</option>)}</select></label><label>Effort<select value={editor.form.effort} onChange={e => setField("effort", e.target.value)}>{PROVIDERS.find(p => p.id === editor.form.provider)?.efforts.map(e => <option key={e} value={e}>{e || "Provider default / in ID"}</option>)}</select></label></div><label>Status<select value={editor.form.status} onChange={e => setField("status", e.target.value)}><option value="current">Current — can be recommended</option><option value="legacy">Legacy — explicit use only</option><option value="restricted">Restricted — additional access required</option></select></label><label>Notes<textarea rows={3} value={editor.form.notes} onChange={e => setField("notes", e.target.value)} placeholder="What is this model good at? Any access requirements?" /></label><details className="admin-advanced"><summary>Measurements & sources</summary><p>Leave unmeasured values empty. API token prices are different from measured task cost.</p><div className="admin-form-pair"><label>Cost per task (USD)<input type="number" min="0" step="any" value={editor.form.price} onChange={e => setField("price", e.target.value)} placeholder="Unmeasured" /></label><label>Seconds per task<input type="number" min="0" step="any" value={editor.form.secs} onChange={e => setField("secs", e.target.value)} placeholder="Unmeasured" /></label></div><label>Benchmark scores (JSON)<textarea className="admin-json" rows={6} value={editor.form.scores} onChange={e => setField("scores", e.target.value)} spellCheck={false} /><small>Use benchmark IDs with measured numeric scores, or null.</small></label><label>Sources<textarea rows={3} value={editor.form.sources} onChange={e => setField("sources", e.target.value)} placeholder="https://…" spellCheck={false} /><small>One source URL per line.</small></label></details>{editorError && <p className="admin-form-error" role="alert">{editorError}</p>}<div className="admin-form-footer"><button className="admin-button admin-primary" type="submit" disabled={editor.identity !== null && !editorDirty}>{editor.identity === null ? "Add to draft" : "Stage model changes"}<Check size={16} /></button><small>Nothing is published yet.</small></div></fieldset></form></aside>}
        </div> : <section className="admin-categories" aria-label="Category recommendations">
          <nav className="admin-category-list" aria-label="Choose a category">{CATEGORIES.map(name => <button type="button" key={name} aria-pressed={category === name} onClick={() => { setCategory(name); setAddPick(""); setCategoryError("") }}><span>{name}</span><small>{catalog.keys[name].picks.length} picks</small><ArrowRight size={15} /></button>)}</nav>
          {activeKey && <div className="admin-category-editor"><div className="admin-panel-heading"><div><span className="admin-eyebrow">Ordered recommendations</span><h2>{category}</h2></div><span className="admin-status current">Draft</span></div><p className="admin-category-intro">The first eligible pick wins. Models that require availability are skipped unless the provider lists them; put their fallback immediately after them.</p><label className="admin-category-description">Description<input value={activeKey.says} disabled={disabled} onChange={e => { if (catalog) { setCatalog({ ...catalog, keys: { ...catalog.keys, [category]: { ...activeKey, says: e.target.value } } }); setCategoryError("") } }} onBlur={() => { if (!activeKey.says.trim()) setCategoryError("Add a description before publishing.") }} /></label><ol className="admin-picks">{activeKey.picks.map((pick, index) => <li key={`${modelIdentity(pick)}-${index}`}><div className="admin-pick-order">{String(index + 1).padStart(2, "0")}</div><div className="admin-pick-main"><div className="admin-pick-title"><strong>{pick.model}</strong>{index === 0 && <span className="admin-default-label">Default</span>}</div><span className="admin-pick-meta">{LABELS[pick.provider]} · {pick.effort || "effort in model ID"}</span>{canRunFast(pick.provider, pick.model) && <label className="admin-fast-toggle"><input type="checkbox" checked={Boolean(pick.fast)} disabled={disabled} onChange={e => updateCategory({ ...activeKey, picks: activeKey.picks.map((p, i) => i === index ? { ...p, fast: e.target.checked } : p) })} />Faster service tier <small>Uses more credits</small></label>}<label className="admin-fast-toggle"><input type="checkbox" checked={Boolean(pick.requires_availability)} disabled={disabled} onChange={e => updateCategory({ ...activeKey, picks: activeKey.picks.map((p, i) => i === index ? { ...p, requires_availability: e.target.checked } : p) })} />Require model availability <small>Otherwise try the next pick</small></label></div><div className="admin-pick-actions"><button type="button" className="admin-icon-button" disabled={disabled || index === 0} aria-label={`Move ${title(pick)} up`} onClick={() => movePick(index, -1)}><ArrowUp size={16} /></button><button type="button" className="admin-icon-button" disabled={disabled || index === activeKey.picks.length - 1} aria-label={`Move ${title(pick)} down`} onClick={() => movePick(index, 1)}><ArrowDown size={16} /></button><button type="button" className="admin-icon-button" disabled={disabled || activeKey.picks.length === 1} aria-label={`Remove ${title(pick)} from ${category}`} onClick={() => updateCategory({ ...activeKey, picks: activeKey.picks.filter((_, i) => i !== index) })}><X size={16} /></button></div></li>)}</ol><div className="admin-add-pick"><label><span>Add a current model</span><select value={addPick} disabled={disabled} onChange={e => setAddPick(e.target.value)}><option value="">Choose a configuration</option>{currentRows.filter(m => !activeKey.picks.some(p => modelIdentity(p) === modelIdentity(m))).map(m => <option key={modelIdentity(m)} value={modelIdentity(m)}>{title(m)} · {LABELS[m.provider]}</option>)}</select></label><button type="button" className="admin-button" disabled={disabled || !addPick} onClick={appendPick}><Plus size={16} />Add fallback</button></div>{categoryError && <p className="admin-form-error" role="alert">{categoryError}</p>}<p className="admin-category-footnote">Only current configurations can be recommended. Add a model in the Models section first.</p></div>}
        </section>}
        <footer className="admin-footnote"><span>Drafts stay in this browser tab. Published changes apply to the shared catalog.</span><button className="admin-text-link" type="button" onClick={() => void reloadLatest()} disabled={Boolean(busy)}>{dirty ? "Download draft & load latest" : "Reload latest catalog"}</button></footer>
      </>}
    </div>
  )
}
