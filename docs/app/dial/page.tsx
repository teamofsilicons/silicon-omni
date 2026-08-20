"use client"

import { useEffect, useMemo, useState } from "react"

import { Footer } from "../footer"
import { PROVIDERS, provider as findProvider } from "../../lib/providers"
import { dialFor, type Entry } from "../../lib/dial"

interface Model {
  name: string
  vendor: string
  context: number | null
  reasoning: boolean
}

const EVERY = ["claude", "openai", "google"]
const blank = { provider: "claude", model: "", effort: "", score: "", price: "", note: "" }

export default function Dial() {
  const [token, setToken] = useState("")
  const [entries, setEntries] = useState<Entry[]>([])
  const [models, setModels] = useState<Model[]>([])
  const [form, setForm] = useState({ ...blank })
  const [busy, setBusy] = useState(false)
  const [says, setSays] = useState("")
  const [wrong, setWrong] = useState(false)
  const [live, setLive] = useState(true)

  useEffect(() => {
    setToken(localStorage.getItem("omni-token") ?? "")
    load()
    fetch("/api/models")
      .then((r) => r.json())
      .then((d) => setModels(d.models ?? []))
      .catch(() => setModels([]))
  }, [])

  function load() {
    fetch("/api/rungs")
      .then((r) => r.json())
      .then((d) => {
        setEntries(d.entries ?? [])
        setLive(d.live !== false)
      })
      .catch(() => setEntries([]))
  }

  function tell(message: string, bad = false) {
    setSays(message)
    setWrong(bad)
  }

  const here = findProvider(form.provider) ?? PROVIDERS[0]

  const suggestions = useMemo(
    () => models.filter((m) => here.vendors.includes(m.vendor)),
    [models, here],
  )

  // where each entry lands on the dial everyone with all three providers sees
  const placed = useMemo(() => {
    const rungs = dialFor(entries, EVERY)
    const at = new Map<string, number[]>()
    for (const [level, rung] of Object.entries(rungs)) {
      const id = `${rung.provider}/${rung.model}/${rung.effort}`
      at.set(id, [...(at.get(id) ?? []), Number(level)])
    }
    return at
  }, [entries])

  async function save(event: React.FormEvent) {
    event.preventDefault()
    setBusy(true)
    tell("")
    try {
      const response = await fetch("/api/rungs", {
        method: "POST",
        headers: { "content-type": "application/json", "x-omni-token": token },
        body: JSON.stringify(form),
      })
      const body = await response.json()
      if (!response.ok) throw new Error(body.error ?? "refused")
      localStorage.setItem("omni-token", token)
      setForm({ ...blank, provider: form.provider })
      tell(`saved ${form.model}`)
      load()
    } catch (problem) {
      tell(String(problem instanceof Error ? problem.message : problem), true)
    } finally {
      setBusy(false)
    }
  }

  async function drop(id: number, name: string) {
    setBusy(true)
    try {
      const response = await fetch(`/api/rungs?id=${id}`, {
        method: "DELETE",
        headers: { "x-omni-token": token },
      })
      const body = await response.json()
      if (!response.ok) throw new Error(body.error ?? "refused")
      tell(`removed ${name}`)
      load()
    } catch (problem) {
      tell(String(problem instanceof Error ? problem.message : problem), true)
    } finally {
      setBusy(false)
    }
  }

  async function seed() {
    setBusy(true)
    try {
      const response = await fetch("/api/rungs", {
        method: "POST",
        headers: { "content-type": "application/json", "x-omni-token": token },
        body: JSON.stringify({ action: "seed" }),
      })
      const body = await response.json()
      if (!response.ok) throw new Error(body.error ?? "refused")
      tell(body.planted ? `planted ${body.planted} models` : "already has models, left alone")
      load()
    } catch (problem) {
      tell(String(problem instanceof Error ? problem.message : problem), true)
    } finally {
      setBusy(false)
    }
  }

  const onDial = entries.filter((e) => e.score !== null && e.price !== null).length

  return (
    <>
      <div className="page-head">
        <div className="crumb">
          <a href="/">silicon omni</a> / dial
        </div>
        <h1>the dial.</h1>
        <p>
          Every model here is a point on a graph: how good it is, and what it costs. Only the left
          edge becomes a dial — a model earns a level if nothing else is both better and cheaper.
          Add one and every omni install picks it up within the hour.
        </p>
      </div>

      <section className="section">
        {!live && (
          <div className="notice bad">
            no database on this deployment — showing the packaged seed, and nothing can be saved.
            add a postgres store and set DATABASE_URL.
          </div>
        )}
        {says && <div className={`notice${wrong ? " bad" : ""}`}>{says}</div>}

        <div className="field" style={{ maxWidth: 420 }}>
          <label htmlFor="token">admin token</label>
          <input
            id="token"
            type="password"
            value={token}
            placeholder="ADMIN_TOKEN"
            onChange={(e) => setToken(e.target.value)}
          />
          <div className="hint">
            Kept in this browser only. Whoever has it decides what every install resolves to.
          </div>
        </div>

        <h3>Add a model</h3>
        <form onSubmit={save}>
          <div className="two-up">
            <div className="field">
              <label htmlFor="provider">provider</label>
              <select
                id="provider"
                value={form.provider}
                onChange={(e) => setForm({ ...form, provider: e.target.value, effort: "" })}
              >
                {PROVIDERS.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.id} — {p.label}
                  </option>
                ))}
              </select>
              <div className="hint">Which CLI runs it.</div>
            </div>

            <div className="field">
              <label htmlFor="effort">effort</label>
              <select
                id="effort"
                value={form.effort}
                onChange={(e) => setForm({ ...form, effort: e.target.value })}
              >
                {here.efforts.map((effort) => (
                  <option key={effort} value={effort}>
                    {effort === "" ? "(none — do not pass the flag)" : effort}
                  </option>
                ))}
              </select>
              <div className="hint">
                What {here.label} accepts. Effort is per provider, not per model.
              </div>
            </div>
          </div>

          <div className="field">
            <label htmlFor="model">model</label>
            <input
              id="model"
              type="text"
              list="models"
              required
              value={form.model}
              placeholder="claude-opus-5"
              onChange={(e) => setForm({ ...form, model: e.target.value })}
            />
            <datalist id="models">
              {suggestions.map((m) => (
                <option key={m.name} value={m.name}>
                  {m.vendor}
                </option>
              ))}
            </datalist>
            <div className="hint">
              Passed to the CLI verbatim. {suggestions.length} names suggested from{" "}
              <a href="https://models.litellm.ai/" target="_blank" rel="noreferrer">
                litellm
              </a>{" "}
              for {here.label}, but a CLI-only slug that no catalogue knows — {" "}
              <code className="inline">gpt-5.6-sol</code>,{" "}
              <code className="inline">gemini-3.7-flash-high</code> — is fine to type in.
            </div>
          </div>

          <div className="two-up">
            <div className="field">
              <label htmlFor="score">gdpval elo</label>
              <input
                id="score"
                type="number"
                step="0.1"
                value={form.score}
                placeholder="1844.7"
                onChange={(e) => setForm({ ...form, score: e.target.value })}
              />
              <div className="hint">
                From{" "}
                <a
                  href="https://artificialanalysis.ai/evaluations/gdpval-aa"
                  target="_blank"
                  rel="noreferrer"
                >
                  GDPval-AA v2
                </a>
                . Leave blank and it is stored but stays off the dial.
              </div>
            </div>
            <div className="field">
              <label htmlFor="price">usd per task</label>
              <input
                id="price"
                type="number"
                step="0.0001"
                value={form.price}
                placeholder="6.766"
                onChange={(e) => setForm({ ...form, price: e.target.value })}
              />
              <div className="hint">Cost per GDPval task on the same leaderboard.</div>
            </div>
          </div>

          <div className="field">
            <label htmlFor="note">note</label>
            <input
              id="note"
              type="text"
              value={form.note}
              placeholder="where these numbers came from"
              onChange={(e) => setForm({ ...form, note: e.target.value })}
            />
          </div>

          <div className="row" style={{ marginTop: 0 }}>
            <button className="btn-dark" type="submit" disabled={busy || !live}>
              {busy ? "saving" : "save model"}
            </button>
            <button className="btn-outline" type="button" onClick={seed} disabled={busy || !live}>
              plant the seed
            </button>
          </div>
        </form>

        <h3>
          On the graph{" "}
          <span className="pill on">
            {onDial} plotted · {entries.length - onDial} waiting for numbers
          </span>
        </h3>
        <p>
          Levels shown are the dial someone with all three providers gets. A model can be on the
          graph and still hold no level, because something else is better <em>and</em> cheaper —
          and it can reappear the moment a provider is unavailable.
        </p>

        <table className="dial-table">
          <thead>
            <tr>
              <th>level</th>
              <th>provider</th>
              <th>model</th>
              <th>effort</th>
              <th>elo</th>
              <th>$/task</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {entries.map((entry) => {
              const at = placed.get(`${entry.provider}/${entry.model}/${entry.effort}`)
              return (
                <tr key={entry.id ?? `${entry.provider}/${entry.model}/${entry.effort}`}>
                  <td className="lvl">
                    {at ? (
                      <span className="pill on">{at.sort((a, b) => b - a).join(" ")}</span>
                    ) : entry.score === null || entry.price === null ? (
                      <span className="pill">no numbers</span>
                    ) : (
                      <span className="pill">shadowed</span>
                    )}
                  </td>
                  <td>{entry.provider}</td>
                  <td>{entry.model}</td>
                  <td>{entry.effort || <span style={{ opacity: 0.35 }}>none</span>}</td>
                  <td>{entry.score ?? "—"}</td>
                  <td>{entry.price ?? "—"}</td>
                  <td>
                    {entry.id && live && (
                      <button
                        className="copy-btn"
                        style={{ color: "var(--ink)", borderColor: "var(--rule)" }}
                        onClick={() => drop(entry.id!, entry.model)}
                        disabled={busy}
                      >
                        remove
                      </button>
                    )}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>

        {!entries.length && <p>Nothing yet. Plant the seed to start from the shipped graph.</p>}
      </section>

      <Footer />
    </>
  )
}
