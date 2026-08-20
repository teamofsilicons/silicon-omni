"use client"

import { useEffect, useMemo, useState } from "react"

import { Footer } from "../footer"
import { PROVIDERS, combinations, provider as findProvider } from "../../lib/providers"
import { dialFor, type Entry } from "../../lib/dial"

interface Model {
  name: string
  vendor: string
}

interface Links {
  raw: string
  edit: string
  view: string
}

const blank = { provider: "claude", model: "", effort: "", score: "", price: "" }

export default function Dial() {
  const [models, setModels] = useState<Entry[]>([])
  const [catalogue, setCatalogue] = useState<Model[]>([])
  const [links, setLinks] = useState<Links | null>(null)
  const [fresh, setFresh] = useState(true)
  const [combo, setCombo] = useState("claude+google+openai")
  const [form, setForm] = useState({ ...blank })
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    fetch("/api/graph")
      .then((r) => r.json())
      .then((d) => {
        setModels(d.models ?? [])
        setLinks(d.links ?? null)
        setFresh(d.fresh !== false)
      })
      .catch(() => setModels([]))
    fetch("/api/models")
      .then((r) => r.json())
      .then((d) => setCatalogue(d.models ?? []))
      .catch(() => setCatalogue([]))
  }, [])

  const here = findProvider(form.provider) ?? PROVIDERS[0]
  const suggestions = useMemo(
    () => catalogue.filter((m) => here.vendors.includes(m.vendor)),
    [catalogue, here],
  )

  const providers = combo.split("+")
  const rungs = useMemo(() => dialFor(models, providers), [models, combo])

  const placed = useMemo(() => {
    const at = new Map<string, number[]>()
    for (const [level, rung] of Object.entries(rungs)) {
      const id = `${rung.provider}/${rung.model}/${rung.effort}`
      at.set(id, [...(at.get(id) ?? []), Number(level)])
    }
    return at
  }, [rungs])

  const snippet = JSON.stringify(
    {
      provider: form.provider,
      model: form.model || "…",
      effort: form.effort,
      ...(form.score ? { score: Number(form.score) } : {}),
      ...(form.price ? { price: Number(form.price) } : {}),
    },
    null,
    2,
  )

  const ready = Boolean(form.model.trim())

  return (
    <>
      <div className="page-head">
        <div className="crumb">
          <a href="/">silicon omni</a> / dial
        </div>
        <h1>the dial.</h1>
        <p>
          Every model is a point on a graph: how good it is, and what it costs. Only the left edge
          becomes a dial — a model earns a level when nothing else is both better and cheaper.
        </p>
        <p>
          There is no database and no login. The list lives in{" "}
          <code className="inline">docs/data/models.json</code> in a public repo. Edit it, commit,
          and every install picks the change up within the hour. No release.
        </p>
        <div className="row">
          {links && (
            <a className="btn-dark" href={links.edit} target="_blank" rel="noreferrer">
              edit on github
            </a>
          )}
          {links && (
            <a className="btn-outline" href={links.raw} target="_blank" rel="noreferrer">
              raw json
            </a>
          )}
          <a className="btn-outline" href={`/api/intelligence?providers=${combo}`}>
            this dial as json
          </a>
        </div>
      </div>

      <section className="section">
        {!fresh && (
          <div className="notice bad">
            could not reach github — showing the copy compiled into this deployment.
          </div>
        )}

        <h2>the dial.</h2>
        <p>
          Pick who is signed in. Losing a provider is a different dial, not a filtered one: a model
          another vendor&apos;s was shadowing comes straight back.
        </p>

        <div className="field" style={{ maxWidth: 420 }}>
          <label htmlFor="combo">providers signed in</label>
          <select id="combo" value={combo} onChange={(e) => setCombo(e.target.value)}>
            {combinations().map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </div>

        <table className="dial-table">
          <thead>
            <tr>
              <th>level</th>
              <th>elo</th>
              <th>$/task</th>
              <th>provider</th>
              <th>model</th>
              <th>effort</th>
            </tr>
          </thead>
          <tbody>
            {Array.from({ length: 11 }, (_, i) => 10 - i).map((level) => {
              const rung = rungs[String(level)]
              if (!rung) return null
              return (
                <tr key={level}>
                  <td className="lvl">{level}</td>
                  <td>{rung.score}</td>
                  <td>{rung.price}</td>
                  <td>{rung.provider}</td>
                  <td>{rung.model}</td>
                  <td>{rung.effort || <span style={{ opacity: 0.35 }}>none</span>}</td>
                </tr>
              )
            })}
          </tbody>
        </table>
        {!Object.keys(rungs).length && <p>Nothing plotted for those providers yet.</p>}

        <h2 style={{ marginTop: "5rem" }}>everything on the graph.</h2>
        <p>
          {models.length} models. A model can sit here holding no level, because something else is
          better <em>and</em> cheaper — that is the graph doing its job, not a mistake.
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
            </tr>
          </thead>
          <tbody>
            {models.map((entry) => {
              const at = placed.get(`${entry.provider}/${entry.model}/${entry.effort}`)
              const unpriced = entry.score == null || entry.price == null
              return (
                <tr key={`${entry.provider}/${entry.model}/${entry.effort}`}>
                  <td>
                    {at ? (
                      <span className="pill on">{at.sort((a, b) => b - a).join(" ")}</span>
                    ) : (
                      <span className="pill">{unpriced ? "no numbers" : "shadowed"}</span>
                    )}
                  </td>
                  <td>{entry.provider}</td>
                  <td>{entry.model}</td>
                  <td>{entry.effort || <span style={{ opacity: 0.35 }}>none</span>}</td>
                  <td>{entry.score ?? "—"}</td>
                  <td>{entry.price ?? "—"}</td>
                </tr>
              )
            })}
          </tbody>
        </table>

        <h2 style={{ marginTop: "5rem" }}>add one.</h2>
        <p>
          This writes nothing. It builds the entry so the three things that cannot be guessed are
          right — which CLI runs it, the exact model string, and an effort that CLI actually
          accepts — then you paste it into the file and commit.
        </p>

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
              Everything {here.label} accepts. Effort is a property of the provider, not the model.
            </div>
          </div>
        </div>

        <div className="field">
          <label htmlFor="model">model</label>
          <input
            id="model"
            type="text"
            list="catalogue"
            value={form.model}
            placeholder="claude-opus-5"
            onChange={(e) => setForm({ ...form, model: e.target.value })}
          />
          <datalist id="catalogue">
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
            for {here.label} — but type freely, because a CLI-only slug like{" "}
            <code className="inline">gpt-5.6-sol</code> or{" "}
            <code className="inline">gemini-3.7-flash-high</code> is in no catalogue.
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
              . Leave blank and the model is recorded but stays off the dial.
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
            <div className="hint">Cost per GDPval task, from the same leaderboard.</div>
          </div>
        </div>

        <div className="terminal">{snippet}</div>

        <div className="row" style={{ marginTop: 0 }}>
          <button
            className="btn-dark"
            disabled={!ready}
            onClick={() => {
              navigator.clipboard.writeText(snippet).then(() => {
                setCopied(true)
                setTimeout(() => setCopied(false), 1600)
              })
            }}
          >
            {copied ? "copied" : "copy the entry"}
          </button>
          {links && (
            <a className="btn-outline" href={links.edit} target="_blank" rel="noreferrer">
              paste it into models.json
            </a>
          )}
        </div>
      </section>

      <Footer />
    </>
  )
}
