import { Logo, type Vendor } from "./logos"

/**
 * One thread, three vendors, and an answer that shows its sources.
 *
 * This was four bordered cards stacked up, each with its own coloured rail, and
 * the borders between them cut the rail into four — so the picture said "four
 * separate exchanges" while the paragraph above it said "one conversation."
 *
 * Now the rail is continuous. Each leg's segment butts against the next, so the
 * line runs unbroken from the first message to the last and simply changes
 * colour where the vendor changes. A dot on the rail marks each handoff.
 *
 * The payoff is the last line. Claude is asked to list all three facts, and the
 * three it returns are printed in the colours of the three companies that were
 * told them — so the answer carries its own provenance, and you can see at a
 * glance that two thirds of it came out of somebody else's model.
 */

const LEGS = [
  {
    who: "claude-code-cli",
    model: "claude-sonnet-5",
    said: "Remember: fact one is RUBY-1.",
    back: "OK",
    token: "RUBY-1",
  },
  {
    who: "codex-app-server",
    model: "gpt-5.4-mini",
    said: "Remember: fact two is JADE-2.",
    back: "OK",
    token: "JADE-2",
  },
  {
    who: "antigravity-cli",
    model: "gemini-3.5-flash-low",
    said: "Remember: fact three is ONYX-3.",
    back: "OK",
    token: "ONYX-3",
  },
]

export function Relay() {
  return (
    <div className="relay">
      {LEGS.map((leg, i) => (
        <div
          key={leg.who}
          className="leg"
          style={{
            ["--tint" as string]: `var(--${leg.who})`,
            ["--tint-ink" as string]: `var(--${leg.who}-ink)`,
          }}
        >
          <div className="leg-head">
            <Logo of={leg.who as Vendor} size={14} />
            <b>{leg.who}</b>
            <span className="leg-model num">{leg.model}</span>
            {i > 0 && <span className="leg-move num">switched</span>}
          </div>
          <p className="leg-said">{leg.said}</p>
          <p className="leg-back">{leg.back}</p>
        </div>
      ))}

      <div
        className="leg leg-end"
        style={{
          ["--tint" as string]: "var(--claude-code-cli)",
          ["--tint-ink" as string]: "var(--claude-code-cli-ink)",
        }}
      >
        <div className="leg-head">
          <Logo of="claude-code-cli" size={14} />
          <b>claude</b>
          <span className="leg-model num">claude-sonnet-5</span>
          <span className="leg-move num">switched back</span>
        </div>
        <p className="leg-said">List all three facts.</p>
        <p className="leg-answer">
          {LEGS.map((leg, i) => (
            <span key={leg.token}>
              <em style={{ ["--tint" as string]: `var(--${leg.who})` }}>{leg.token}</em>
              {i < LEGS.length - 1 ? <i>,</i> : null}
            </span>
          ))}
        </p>
        <p className="leg-note num">
          one of those three was told to this model. the other two were told to somebody else&apos;s.
        </p>
      </div>
    </div>
  )
}
