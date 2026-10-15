"use client"

/**
 * One file, and two columns of dots.
 *
 * This is the mechanic the whole library rests on and the one nobody ever
 * draws. There is a single log with a single sequence number that only goes up,
 * and a second small file that remembers, per provider, the last number that
 * provider has actually been shown. Everything else — the switching, the
 * resuming, the not-repeating-yourself — falls out of those two facts.
 *
 * So: the log on the right, and on the left a column per provider where a
 * filled dot means "this one has seen this line." Watch it play. The columns
 * fill at different times and in different amounts, and at the moment of a
 * switch you can see the exact block of history that gets handed over. Coming
 * back fills a smaller block than going out did, because the first six lines
 * are already in that provider's own session and it is resuming it.
 *
 * The two rungs are passed in from the live dial rather than typed here. They
 * used to be hardcoded, and they had quietly inverted against the ladder two
 * chapters down the same page — the figure explaining the mechanism was
 * contradicting the data explaining the figure. Numbers that can disagree
 * eventually do.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react"

type Who = "hi" | "lo"

/** A rung of the real dial, as the page found it a moment ago. */
export type Held = { level: number; provider: string; model: string; effort: string }

type Row = {
  seq: number
  who: Who | null
  kind: string
  body: string
  code?: boolean
  /** at this line, this provider has now been shown everything up to here */
  syncs?: Who
  /** a switch: which provider arrives, and the block of log it is handed */
  hands?: { to: Who; from: number; keeps?: number }
  turn: number
}

function script(high: Held, low: Held): Row[] {
  return [
    { seq: 0, who: "hi", kind: "config", body: `launch · level ${high.level}`, turn: 0 },
    { seq: 1, who: "hi", kind: "start", body: "Which test in this repo is slowest?", turn: 1 },
    { seq: 2, who: "hi", kind: "thinking", body: "—", turn: 1 },
    { seq: 3, who: "hi", kind: "tool.call", body: "[Bash: pytest --durations=3]", code: true, turn: 1 },
    { seq: 4, who: "hi", kind: "tool.result", body: "[Bash result: 3 slowest durations]", code: true, turn: 1 },
    { seq: 5, who: "hi", kind: "text", body: "test_conversation_survives_a_switch — 41.2s.", turn: 1 },
    { seq: 6, who: "hi", kind: "end", body: "turn complete", syncs: "hi", turn: 1 },

    { seq: 7, who: null, kind: "config", body: `intelligence(${low.level})`, turn: 2 },
    { seq: 8, who: "lo", kind: "switch", body: `the conversation moves to ${low.provider}`, hands: { to: "lo", from: 0 }, turn: 2 },
    { seq: 9, who: "lo", kind: "start", body: "Why is that one slow?", turn: 3 },
    { seq: 10, who: "lo", kind: "thinking", body: "—", turn: 3 },
    { seq: 11, who: "lo", kind: "text", body: "It starts two CLIs and waits out both cold starts before the first token.", turn: 3 },
    { seq: 12, who: "lo", kind: "end", body: "turn complete", syncs: "lo", turn: 3 },

    { seq: 13, who: null, kind: "config", body: `intelligence(${high.level})`, turn: 4 },
    { seq: 14, who: "hi", kind: "switch", body: `back to ${high.provider}`, hands: { to: "hi", from: 7, keeps: 6 }, turn: 4 },
    { seq: 15, who: "hi", kind: "start", body: "Fix it. Start them in parallel.", turn: 5 },
    { seq: 16, who: "hi", kind: "tool.call", body: "[Edit: tests/conftest.py]", code: true, turn: 5 },
    { seq: 17, who: "hi", kind: "text", body: "Both runners start together now. 41.2s → 18.9s.", turn: 5 },
    { seq: 18, who: "hi", kind: "end", body: "turn complete", syncs: "hi", turn: 5 },
  ]
}

/** How far each provider has been shown, once the log has played to `upto`. */
function seen(log: Row[], upto: number): Record<Who, number> {
  const at: Record<Who, number> = { hi: -1, lo: -1 }
  for (const row of log.slice(0, upto + 1)) {
    if (row.hands) at[row.hands.to] = Math.max(at[row.hands.to], row.seq - 1)
    if (row.syncs) at[row.syncs] = row.seq
  }
  return at
}

const HOLD: Record<string, number> = {
  config: 720,
  start: 640,
  thinking: 900,
  "tool.call": 900,
  "tool.result": 620,
  text: 1500,
  end: 900,
  switch: 2100,
}

const name = (r: Held) => `${r.model}${r.effort ? ` ${r.effort}` : ""}`

export function Tape({ high, low }: { high: Held; low: Held }) {
  const log = useMemo(() => script(high, low), [high, low])
  const who = useMemo(
    () => [
      { id: "hi" as Who, tint: high.provider, label: high.provider, model: name(high), level: high.level },
      { id: "lo" as Who, tint: low.provider, label: low.provider, model: name(low), level: low.level },
    ],
    [high, low],
  )

  /* Server-rendered at the last frame, so the two columns — the entire point of
     the figure — are full rather than empty for a reader with no script. The
     effect below rewinds to the start and plays it properly. */
  const [at, setAt] = useState(log.length - 1)
  const [playing, setPlaying] = useState(true)
  const [live, setLive] = useState(false)
  const hull = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      setPlaying(false)
      return
    }
    setAt(-1)
  }, [])

  useEffect(() => {
    const el = hull.current
    if (!el) return
    const eye = new IntersectionObserver(([entry]) => setLive(entry.isIntersecting), { threshold: 0.2 })
    eye.observe(el)
    return () => eye.disconnect()
  }, [])

  useEffect(() => {
    if (!playing || !live) return
    const next = at + 1
    if (next >= log.length) {
      const rest = setTimeout(() => setAt(-1), 3600)
      return () => clearTimeout(rest)
    }
    const wait = at < 0 ? 420 : HOLD[log[at].kind] ?? 700
    const timer = setTimeout(() => setAt(next), wait)
    return () => clearTimeout(timer)
  }, [at, playing, live, log])

  const step = useCallback(
    (by: number) => {
      setPlaying(false)
      setAt((n) => Math.max(-1, Math.min(log.length - 1, n + by)))
    },
    [log.length],
  )

  const read = seen(log, at)
  const now = at >= 0 ? log[at] : null

  return (
    <div
      className="tape"
      ref={hull}
      tabIndex={0}
      role="group"
      aria-label="One session log, and how far each provider has been shown"
      onKeyDown={(e) => {
        if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); step(1) }
        if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); step(-1) }
        if (e.key === " ") { e.preventDefault(); setPlaying((p) => !p) }
      }}
    >
      <div className="tape-head">
        <span className="tape-file num">~/.omni/sessions/nightly.jsonl</span>
        <div className="tape-keys">
          {who.map((w) => (
            <span key={w.id} className="tape-key num" style={{ ["--tone" as string]: `var(--${w.tint})` }}>
              <i />
              {w.label}
              <u>{read[w.id] < 0 ? "waiting" : `read to ${read[w.id]}`}</u>
            </span>
          ))}
        </div>
      </div>

      <ol className="tape-log">
        <li className="tape-row heads" aria-hidden>
          <span className="tape-seq num">seq</span>
          <span className="tape-cells">
            {who.map((w) => (
              <em key={w.id}>{w.label.slice(0, 2)}</em>
            ))}
          </span>
          <span className="tape-kind num">event</span>
          <span className="tape-body">what was recorded</span>
        </li>
        {log.map((row) => {
          const here = row.seq <= at
          const isNow = at === row.seq
          return (
            <li
              key={row.seq}
              className={`tape-row${here ? " here" : ""}${isNow ? " now" : ""}${row.turn % 2 ? " band" : ""}${row.hands ? " hand" : ""}`}
            >
              <span className="tape-seq num">{String(row.seq).padStart(2, "0")}</span>
              <span className="tape-cells">
                {who.map((w) => {
                  const has = here && row.seq <= read[w.id]
                  const justNow = Boolean(now?.hands && now.hands.to === w.id && row.seq >= now.hands.from && row.seq < now.seq)
                  const kept = Boolean(now?.hands?.keeps !== undefined && now.hands.to === w.id && row.seq <= now.hands.keeps!)
                  return (
                    <i
                      key={w.id}
                      className={`tape-cell${has ? " full" : ""}${justNow ? " fresh" : ""}${kept ? " kept" : ""}`}
                      style={{ ["--tone" as string]: `var(--${w.tint})` }}
                    />
                  )
                })}
              </span>
              <span className="tape-kind num">{row.kind}</span>
              <span className="tape-body">
                {row.kind === "thinking" ? (
                  <span className="tape-dim">the model is reasoning. we record that it did.</span>
                ) : row.code ? (
                  <code>{row.body}</code>
                ) : row.kind === "end" || row.kind === "config" ? (
                  <span className="tape-dim">{row.body}</span>
                ) : (
                  row.body
                )}
                {row.hands && (
                  <em className="tape-hand num">
                    {row.hands.keeps !== undefined
                      ? `handed ${row.hands.from}–${row.seq - 1} · resumes its own session, already holds 0–${row.hands.keeps}`
                      : `handed ${row.hands.from}–${row.seq - 1} · ${row.seq - row.hands.from} events, each one once`}
                  </em>
                )}
              </span>
            </li>
          )
        })}
      </ol>

      <div className="tape-foot">
        <button className="tape-btn num" onClick={() => setPlaying((p) => !p)} aria-pressed={playing}>
          {playing ? "pause" : at >= log.length - 1 ? "again" : "play"}
        </button>
        <button className="tape-btn num" onClick={() => step(-1)} aria-label="previous line">←</button>
        <button className="tape-btn num" onClick={() => step(1)} aria-label="next line">→</button>
        <span className="tape-note num">
          {at >= 0 ? `seq ${at} of ${log.length - 1}` : "one file, both of them"}
        </span>
      </div>
    </div>
  )
}
