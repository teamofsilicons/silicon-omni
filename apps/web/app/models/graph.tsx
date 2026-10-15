"use client"

/**
 * The whole board, and what it answers.
 *
 * Turn a provider off and the answer changes, because the models you can run
 * are the only models in the comparison. Change the benchmark and the board
 * itself changes shape — a model that owns one is mid-table on the next, which
 * is the entire reason the registry asks what the work is before it answers.
 *
 * Everything here is the same arithmetic /choose.json runs, from the same
 * file, so this page cannot drift from what the library is told.
 */

import { useMemo, useRef, useState } from "react"

import { edge, plottable, type Entry, type Model } from "../../lib/choose"
import { PROVIDERS } from "../../lib/providers"
import { sample, on as inkOn } from "../../lib/ramp"
import { Logo, MARK, type Vendor } from "../logos"
import { Tip } from "../tip"

const W = 1000
const H = 620
const PAD = { top: 56, right: 40, bottom: 64, left: 64 }

const key = (m: { provider: string; model: string; effort: string }) =>
  `${m.provider}/${m.model}/${m.effort}`
const money = (n: number) => (n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`)
const clock = (s: number) =>
  s >= 60 ? `${Math.floor(s / 60)}m ${String(Math.round(s % 60)).padStart(2, "0")}s` : `${Math.round(s)}s`

const TINT: Record<string, string> = {
  "claude-code-cli": "var(--claude-code-cli)",
  "codex-app-server": "var(--codex-app-server)",
  "antigravity-cli": "var(--antigravity-cli)",
}

function Glyph({ of, cx, cy, r, className }: { of: string; cx: number; cy: number; r: number; className?: string }) {
  const d = MARK[of as Vendor]
  if (!d) return <circle cx={cx} cy={cy} r={r} className={className} />
  const k = (r * 2) / 24
  return (
    <g transform={`translate(${cx - r} ${cy - r}) scale(${k})`} className={className}>
      <path d={d} />
    </g>
  )
}

export function Graph({
  models,
  benches,
}: {
  models: Entry[]
  benches: Record<string, { label: string; short: string; url: string }>
}) {
  const names = Object.keys(benches)
  const [bench, setBench] = useState(names.includes("gdpval-aa") ? "gdpval-aa" : names[0])
  const [off, setOff] = useState<string[]>([])
  const [held, setHeld] = useState<Model | null>(null)
  const nudge = useRef(new Map<string, [number, number]>())

  const on: string[] = PROVIDERS.map((p) => p.id as string).filter((id) => !off.includes(id))

  const ranked = useMemo(
    () => plottable(models, bench).filter((m) => on.includes(m.provider)),
    [models, bench, on.join(",")],
  )
  /* The left edge, best first: nothing else is both better and cheaper. */
  const best = useMemo(() => {
    const current = new Set(models.filter(m => m.status !== "legacy" && m.status !== "restricted").map(m => key({ ...m, effort: m.effort ?? "" } as Model)))
    return edge(ranked.filter(m => current.has(key(m))), bench)
  }, [ranked, bench, models])
  const bestKeys = useMemo(() => new Set(best.map(key)), [best])

  const bounds = useMemo(() => {
    if (!ranked.length) return { x0: -2, x1: 1, y0: 0, y1: 1 }
    const prices = ranked.map((p) => Math.log10(Math.max(p.price, 1e-4)))
    const scores = ranked.map((p) => p.scores[bench])
    const lo = Math.min(...scores)
    const hi = Math.max(...scores)
    const air = (hi - lo) * 0.12 || 1
    return { x0: Math.min(...prices) - 0.2, x1: Math.max(...prices) + 0.2, y0: lo - air, y1: hi + air }
  }, [ranked, bench])

  const x = (price: number) =>
    PAD.left +
    ((Math.log10(Math.max(price, 1e-4)) - bounds.x0) / (bounds.x1 - bounds.x0)) *
      (W - PAD.left - PAD.right)
  const y = (score: number) =>
    PAD.top + (H - PAD.top - PAD.bottom) -
    ((score - bounds.y0) / (bounds.y1 - bounds.y0)) * (H - PAD.top - PAD.bottom)

  const decades = useMemo(() => {
    const out: number[] = []
    for (let e = Math.ceil(bounds.x0); e <= Math.floor(bounds.x1); e++) out.push(10 ** e)
    return out
  }, [bounds])

  const scores = useMemo(() => {
    const lo = bounds.y0
    const hi = bounds.y1
    return [lo + (hi - lo) * 0.15, lo + (hi - lo) * 0.5, lo + (hi - lo) * 0.85]
  }, [bounds])

  const round = (n: number) => (n >= 100 ? Math.round(n) : Number(n.toFixed(n >= 10 ? 1 : 2)))

  return (
    <div className="graph">
      <div className="graph-toggles">
        {names.map((id) => (
          <button
            key={id}
            className={`toggle${id === bench ? " on" : ""}`}
            onClick={() => setBench(id)}
            aria-pressed={id === bench}
          >
            {benches[id]?.short ?? id}
          </button>
        ))}
      </div>

      <div className="graph-toggles">
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            className={`toggle${off.includes(p.id) ? "" : " on"}`}
            style={{ ["--tint" as string]: TINT[p.id] }}
            onClick={() => setOff((was) => (was.includes(p.id) ? was.filter((v) => v !== p.id) : [...was, p.id]))}
            aria-pressed={!off.includes(p.id)}
          >
            <Logo of={p.id as Vendor} size={14} />
            {p.label}
          </button>
        ))}
        <span className="graph-count num">
          {ranked.length} scored on <b>{benches[bench]?.label}</b>
        </span>
      </div>


      <svg className="graph-svg" viewBox={`0 0 ${W} ${H}`} role="img"
        aria-label={`${ranked.length} models plotted by ${benches[bench]?.label} against cost`}>
        {decades.map((t, i) => (
          <g key={t}>
            <line x1={x(t)} x2={x(t)} y1={PAD.top - 10} y2={H - PAD.bottom} className="grid" />
            <text x={x(t)} y={PAD.top - 18} className="tick mid">
              {t >= 1 ? `$${t}` : `$${t.toFixed(t < 0.01 ? 3 : 2)}`}
            </text>
          </g>
        ))}
        {scores.map((s) => (
          <g key={s}>
            <line x1={PAD.left} x2={W - PAD.right} y1={y(s)} y2={y(s)} className="grid" />
            <text x={PAD.left - 10} y={y(s) + 3} className="tick end quiet">
              {round(s)}
            </text>
          </g>
        ))}
        <text x={PAD.left - 10} y={PAD.top - 18} className="tick end">
          {bench} ↑
        </text>

        {ranked.map((p) => {
          const mine = bestKeys.has(key(p))
          const place = best.findIndex((b) => key(b) === key(p))
          const level = place < 0 ? 0 : Math.round(((best.length - 1 - place) * 10) / Math.max(1, best.length - 1))
          const dim = held && !mine && key(held) !== key(p)
          const tone = mine ? sample(level / 10) : undefined
          const cx = x(p.price)
          const cy = y(p.scores[bench])
          return (
            <g
              key={key(p)}
              className={`point${mine ? " on" : ""}${dim ? " dim" : ""}`}
              style={{ ["--tint" as string]: TINT[p.provider], ...(tone ? { ["--tone" as string]: tone } : {}) }}
              onMouseEnter={() => setHeld(p)}
              onMouseLeave={() => setHeld(null)}
            >
              {mine ? (
                <>
                  <circle cx={cx} cy={cy} r={12.5} className="ring" />
                  <circle cx={cx} cy={cy} r={11} className="mark" />
                  <text x={cx} y={cy} className="level" style={{ ["--on" as string]: inkOn(tone!) }}>
                    {level}
                  </text>
                </>
              ) : (
                <Glyph of={p.provider} cx={cx} cy={cy} r={7} className="mark logo-mark" />
              )}
              <circle cx={cx} cy={cy} r={17} className="hit" />
            </g>
          )
        })}

        {held && (
          <Tip
            at={{ x: x(held.price), y: y(held.scores[bench]) }}
            box={{ w: W, h: H }}
            model={held.model}
            effort={held.effort}
            foot={`${round(held.scores[bench])} · ${money(held.price)} · ${clock(held.secs)}`}
          />
        )}
      </svg>

      <div className={`graph-read${held || best.length ? " showing" : ""}`} aria-live="polite">
        {best.length ? (
          <>
            <span className="graph-model">
              {best[0].model}
              {best[0].effort ? <em> {best[0].effort}</em> : null}
            </span>
            <span className="graph-nums">
              {round(best[0].scores[bench])} on {benches[bench]?.label} · {money(best[0].price)} ·{" "}
              {clock(best[0].secs)}
            </span>
            <span className="graph-verdict">
              level 10 of {best.length} rungs · {ranked.length} plotted
            </span>
          </>
        ) : (
          <span className="graph-hint">
            nothing here is scored on {benches[bench]?.label}. turn a provider back on.
          </span>
        )}
      </div>
    </div>
  )
}
