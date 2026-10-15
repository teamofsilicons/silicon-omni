"use client"

/**
 * The field.
 *
 * There is a version of this page that opens with a screenshot and a sentence
 * about unifying your AI workflow. This is not it. Every model omni can reach
 * is one dot, placed by what it scores on the benchmark you picked against
 * what it measurably cost to score it — the whole board, drawn, before a word
 * of it is claimed.
 *
 * Two things are true of this picture and neither can be said as well:
 *
 *   the board changes shape when you change what the work is. A model that
 *   owns one benchmark is mid-table on the next, which is the entire reason
 *   the registry asks what you are doing before it answers;
 *
 *   the three you are shown are not the three at the top. They are the three
 *   that fit what you said you valued, and the marks move when you say
 *   something else.
 *
 * It plays itself, benchmark by benchmark, until a pointer arrives.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import { edge, plottable, type Entry, type Model } from "../lib/choose"
import { sample, on as inkOn } from "../lib/ramp"

type Placed = Model & { x: number; y: number }
import { Tip } from "./tip"

const money = (n: number) => (n >= 1 ? `$${n.toFixed(2)}` : `$${n.toFixed(4)}`)
const clock = (s: number) =>
  s >= 60 ? `${Math.floor(s / 60)}m ${String(Math.round(s % 60)).padStart(2, "0")}s` : `${Math.round(s)}s`
const key = (m: { provider: string; model: string; effort: string }) =>
  `${m.provider}/${m.model}/${m.effort}`

/** how close the pointer has to be before a dot notices */
const REACH = 90

export function Field({
  models,
  benches,
}: {
  models: Entry[]
  benches: Record<string, { label: string; short: string; url: string }>
}) {
  const names = useMemo(() => Object.keys(benches), [benches])
  const [at, setAt] = useState(0)
  const [held, setHeld] = useState<Placed | null>(null)
  const [near, setNear] = useState(false)
  const [box, setBox] = useState({ w: 900, h: 380 })

  const plot = useRef<HTMLDivElement>(null)
  const marks = useRef(new Map<string, SVGCircleElement>())
  const cursor = useRef<{ x: number; y: number } | null>(null)
  const frame = useRef(0)

  const bench = names[at % (names.length || 1)] ?? names[0]

  /* Measure the plot, never the thing the plot is inside: the svg carries an
     intrinsic aspect ratio from its own viewBox, so measuring its parent makes
     each frame taller than the last, forever. */
  useEffect(() => {
    const el = plot.current
    if (!el) return
    const watch = new ResizeObserver(([entry]) => {
      const r = entry.contentRect
      setBox({ w: Math.max(240, Math.round(r.width)), h: Math.max(200, Math.round(r.height)) })
    })
    watch.observe(el)
    return () => watch.disconnect()
  }, [])

  /* It walks the benchmarks until somebody arrives. A hero that only performs
     on hover performs for nobody on a phone. */
  useEffect(() => {
    if (near || !names.length) return
    if (typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches)
      return
    const t = setTimeout(() => setAt((n) => n + 1), 3400)
    return () => clearTimeout(t)
  }, [at, near, names.length])

  const wide = box.w > 760
  const pad = wide
    ? { top: 52, right: 18, bottom: 46, left: 18 }
    : { top: 30, right: 14, bottom: 40, left: 14 }

  const ranked = useMemo(() => plottable(models, bench), [models, bench])
  /* Keep historical dots, but build the recommended edge from current models. */
  const walk = useMemo(() => {
    const current = new Set(models.filter(m => m.status !== "legacy" && m.status !== "restricted").map(m => key({ ...m, effort: m.effort ?? "" } as Model)))
    return edge(ranked.filter(m => current.has(key(m))), bench)
  }, [ranked, bench, models])
  const best = useMemo(() => new Set(walk.map(key)), [walk])

  const bounds = useMemo(() => {
    if (!ranked.length) return { x0: -2, x1: 1, y0: 0, y1: 1 }
    const prices = ranked.map((p) => Math.log10(Math.max(p.price, 1e-4)))
    const scores = ranked.map((p) => p.scores[bench])
    const lo = Math.min(...scores)
    const hi = Math.max(...scores)
    const air = (hi - lo) * 0.12 || 1
    return {
      x0: Math.min(...prices) - 0.18,
      x1: Math.max(...prices) + 0.18,
      y0: lo - air,
      y1: hi + air,
    }
  }, [ranked, bench])

  const atX = useCallback(
    (price: number) =>
      pad.left +
      ((Math.log10(Math.max(price, 1e-4)) - bounds.x0) / (bounds.x1 - bounds.x0)) *
        (box.w - pad.left - pad.right),
    [bounds, box.w, pad.left, pad.right],
  )
  const atY = useCallback(
    (score: number) =>
      pad.top +
      (box.h - pad.top - pad.bottom) -
      ((score - bounds.y0) / (bounds.y1 - bounds.y0)) * (box.h - pad.top - pad.bottom),
    [bounds, box.h, pad.top, pad.bottom],
  )

  const placed = useMemo(
    () => ranked.map((m) => ({ ...m, x: atX(m.price), y: atY(m.scores[bench]) })),
    [ranked, bench, atX, atY],
  )

  /* Proximity is written straight to the DOM. Forty dots at sixty frames is
     nothing, but forty dots through React state on every mouse twitch is a
     re-render per twitch for no gain. */
  useEffect(() => {
    if (!plot.current) return
    const tick = () => {
      const c = cursor.current
      let closest: Placed | null = null
      let bestSoFar = REACH
      for (const p of placed) {
        const node = marks.current.get(key(p))
        if (!node) continue
        if (!c) {
          node.style.setProperty("--pull", "0")
          continue
        }
        const d = Math.hypot(p.x - c.x, p.y - c.y)
        node.style.setProperty("--pull", String(Math.max(0, 1 - d / REACH) ** 1.6))
        if (d < bestSoFar) {
          bestSoFar = d
          closest = p
        }
      }
      setHeld((was) => (was && closest && key(was) === key(closest) ? was : closest))
      frame.current = requestAnimationFrame(tick)
    }
    frame.current = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame.current)
  }, [placed])

  const decades = useMemo(() => {
    const out: number[] = []
    for (let e = Math.ceil(bounds.x0); e <= Math.floor(bounds.x1); e++) out.push(10 ** e)
    return out
  }, [bounds])

  const rankAt = (p: Model) => {
    const i = walk.findIndex((w) => key(w) === key(p))
    return i < 0 ? 0 : Math.round(((walk.length - 1 - i) * 10) / Math.max(1, walk.length - 1))
  }

  const money0 = (n: number) => (n >= 1 ? `$${n}` : `$${n.toFixed(n < 0.01 ? 3 : 2)}`)
  const lit = held ?? placed.find((p) => best.has(key(p))) ?? null

  return (
    <div
      className="field"
      onPointerMove={(e) => {
        const r = plot.current?.getBoundingClientRect()
        if (!r) return
        cursor.current = { x: e.clientX - r.left, y: e.clientY - r.top }
        setNear(true)
      }}
      onPointerLeave={() => {
        cursor.current = null
        setNear(false)
        setHeld(null)
      }}
    >
      <div className="field-plot" ref={plot}>
        <svg
          className="field-svg"
          viewBox={`0 0 ${box.w} ${box.h}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={`${placed.length} models omni can run, plotted by ${benches[bench]?.label} against cost.`}
        >
          {decades.map((t, i) => {
            const end = i === decades.length - 1
            return (
              <g key={t}>
                <line
                  x1={atX(t)}
                  x2={atX(t)}
                  y1={pad.top - 14}
                  y2={box.h - pad.bottom}
                  className="field-grid"
                />
                <text
                  x={atX(t) + (i === 0 ? 4 : end ? -4 : 0)}
                  y={pad.top - 20}
                  className={`field-tick${i === 0 || end ? "" : " mid"}`}
                  textAnchor={i === 0 ? "start" : end ? "end" : undefined}
                >
                  {money0(t)}
                </text>
              </g>
            )
          })}

          {wide && (
            <text x={13} y={pad.top - 20} className="field-axis">
              cost per task →
            </text>
          )}
          <text
            className="field-axis"
            transform={`translate(13 ${pad.top + (box.h - pad.top - pad.bottom) / 2}) rotate(-90)`}
            textAnchor="middle"
          >
            {benches[bench]?.short ?? bench} →
          </text>

          {placed.map((p) => {
            const mine = best.has(key(p))
            return mine ? (
              <g key={key(p)} className="point on">
                <circle cx={p.x} cy={p.y} r={12.5} className="field-mark ring" />
                <circle
                  ref={(node) => {
                    if (node) marks.current.set(key(p), node)
                    else marks.current.delete(key(p))
                  }}
                  cx={p.x}
                  cy={p.y}
                  r={11}
                  className="field-mark"
                  style={{ ["--tone" as string]: sample(rankAt(p) / 10) }}
                />
                <text
                  x={p.x}
                  y={p.y}
                  className="field-level"
                  style={{ ["--tone" as string]: inkOn(sample(rankAt(p) / 10)) }}
                >
                  {rankAt(p)}
                </text>
              </g>
            ) : (
              <circle
                key={key(p)}
                ref={(node) => {
                  if (node) marks.current.set(key(p), node)
                  else marks.current.delete(key(p))
                }}
                cx={p.x}
                cy={p.y}
                r={3.4}
                className={`field-dot${held && key(held) === key(p) ? " lit" : ""}`}
              />
            )
          })}

          {lit && (
            <Tip
              at={{ x: atX(lit.price), y: atY(lit.scores[bench]) }}
              box={box}
              model={lit.model}
              effort={lit.effort}
              foot={`${money(lit.price)} · ${clock(lit.secs)}`}
            />
          )}
        </svg>
      </div>

      <p className="field-read num" aria-live="polite">
        <span className="field-badge" style={{ ["--tone" as string]: "var(--fill-1)", ["--on" as string]: "var(--ink)" }}>
          {benches[bench]?.label}
        </span>
        <span className="field-verdict">
          {placed.length} scored on it · {walk.length} where nothing is both better and cheaper
        </span>
      </p>
    </div>
  )
}
