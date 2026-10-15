"use client"

/**
 * The three ways to ask, as a thing you can actually use.
 *
 * A key is a shortlist somebody chose, and the first vendor on it that you are
 * signed into answers. Turn a provider off and watch the answer walk down the
 * list — that, and not an argument about weights, is what losing a vendor
 * costs you.
 *
 * The number follows the same curated categories as /choose.json, so the
 * preview and the library use the same current model recommendations.
 */

import { useMemo, useState } from "react"

import { byCuratedIntelligence, byKey, type Key } from "../lib/choose"
import { invocation, PROVIDERS } from "../lib/providers"
import { Logo, type Vendor } from "./logos"
import { dial as levelColour, on as inkOn } from "../lib/ramp"

export function Picker({
  keys,
}: {
  keys: Record<string, Key>
}) {
  const names = Object.keys(keys)
  const [how, setHow] = useState<"key" | "intelligence">("key")
  const [key, setKey] = useState(names.includes("code") ? "code" : names[0])
  const [level, setLevel] = useState(7)
  const [off, setOff] = useState<string[]>([])

  const providers = PROVIDERS.map((p) => p.id as string).filter((id) => !off.includes(id))

  const byKeyNow = useMemo(() => byKey(keys, key, providers), [keys, key, providers.join(",")])
  const byDial = useMemo(
    () => byCuratedIntelligence(keys, providers, level),
    [keys, providers.join(","), level],
  )

  const pick = how === "key" ? byKeyNow?.pick ?? null : byDial.pick
  const tone = levelColour(how === "key" ? 7 : level)

  return (
    <div className="picker">
      <div className="picker-how" role="group" aria-label="how to ask">
        <button className={`levers-tab num${how === "key" ? " on" : ""}`} onClick={() => setHow("key")} aria-pressed={how === "key"}>
          a key
        </button>
        <button className={`levers-tab num${how === "intelligence" ? " on" : ""}`} onClick={() => setHow("intelligence")} aria-pressed={how === "intelligence"}>
          a number
        </button>
        <span className="levers-board num">
          {how === "key" ? byKeyNow?.says : `curated ${byDial.category} recommendation`}
        </span>
      </div>

      {how === "key" ? (
        <div className="picker-keys">
          {names.map((id) => (
            <button
              key={id}
              className={`picker-key num${id === key ? " on" : ""}`}
              onClick={() => setKey(id)}
              aria-pressed={id === key}
            >
              {id}
            </button>
          ))}
        </div>
      ) : (
        <label className="lever picker-dial">
          <span className="lever-name num">intelligence</span>
          <input type="range" min={0} max={10} value={level} onChange={(e) => setLevel(Number(e.target.value))} />
          <output className="lever-at num">{level}</output>
        </label>
      )}

      <div className="picker-who">
        {PROVIDERS.map((p) => (
          <button
            key={p.id}
            className={`toggle${off.includes(p.id) ? "" : " on"}`}
            style={{ ["--tint" as string]: `var(--${p.id})` }}
            onClick={() => setOff((was) => (was.includes(p.id) ? was.filter((v) => v !== p.id) : [...was, p.id]))}
            aria-pressed={!off.includes(p.id)}
          >
            <Logo of={p.id as Vendor} size={14} />
            {p.label}
          </button>
        ))}
      </div>

      <div className="levers-say" aria-live="polite">
        {pick ? (
          <>
            <span className="levers-badge num" style={{ background: tone, color: inkOn(tone) }}>
              <Logo of={pick.provider as Vendor} size={11} />
              {pick.provider}
            </span>
            <b>{pick.model}</b>
            {pick.effort ? <em>{pick.effort}</em> : null}
            {pick.fast ? <em className="picker-fast num">fast</em> : null}
          </>
        ) : (
          <span className="levers-nums num">
            nothing on this list is signed in. turn a provider back on.
          </span>
        )}
      </div>

      <pre className="levers-run num">{pick ? invocation(pick) : "—"}</pre>

      {how === "key" && byKeyNow && (
        <p className="levers-next num">
          {byKeyNow.shortlist.map((p, i) => (
            <span key={`${p.provider}/${p.model}/${p.effort}`} className={p.available ? "" : "picker-gone"}>
              {i > 0 ? " › " : "in order: "}
              {p.provider}
              {p.fast ? " fast" : ""}
            </span>
          ))}
        </p>
      )}
    </div>
  )
}
