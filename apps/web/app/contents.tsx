/**
 * The contents, shown rather than listed.
 *
 * Eight chapters is a lot to ask anybody to scroll on trust, and a list of
 * links is a promise with no evidence attached. So each entry carries a working
 * miniature of the thing that chapter argues — built out of the same tokens as
 * the full-size version, at eleven pixels, and clipped to exactly the same
 * height with exactly the same fade. The frame does the unifying, which is why
 * six unlike diagrams read as one set without a scrap of image treatment.
 *
 * The heading of each is the question a reader actually arrives with, not the
 * name of the feature that answers it. The line underneath is the mechanism.
 */

import { sample } from "../lib/ramp"
import { Tokens } from "./code"

/* Laid out for a 470x132 window, so it fills the wide tile at its own aspect
   rather than being stretched to fit one. Down at the old price, then left at
   the new score — the same corner the full-size chart turns. */
const RUNGS: [number, number, number][] = [
  [36, 114, 0], [95, 92, 2], [168, 66, 4], [274, 42, 6], [375, 20, 8],
]

const STAIR = [
  [36, 114], [95, 114], [95, 92], [168, 92], [168, 66], [274, 66],
  [274, 42], [375, 42], [375, 20], [448, 20],
]

const CLOUD = [
  [134, 104], [193, 96], [213, 82], [246, 88], [288, 74], [300, 58], [333, 62],
  [353, 48], [400, 40], [420, 54], [437, 34], [252, 108], [344, 86], [154, 118],
  [384, 66], [445, 50],
]

function MiniFrontier() {
  return (
    <svg viewBox="0 0 470 132" className="mini-svg" preserveAspectRatio="xMinYMin meet" aria-hidden>
      <defs>
        <linearGradient id="mini-ramp" x1="0" y1="1" x2="0.4" y2="0">
          <stop offset="0%" stopColor="#76d4f0" />
          <stop offset="20%" stopColor="#f9f987" />
          <stop offset="60%" stopColor="#fead75" />
          <stop offset="80%" stopColor="#f15347" />
          <stop offset="100%" stopColor="#14245f" />
        </linearGradient>
      </defs>
      {CLOUD.map(([x, y]) => (
        <circle key={`${x}-${y}`} cx={x} cy={y} r={2.4} fill="var(--dot)" />
      ))}
      <polyline
        points={STAIR.map(([x, y]) => `${x},${y}`).join(" ")}
        fill="none"
        stroke="url(#mini-ramp)"
        strokeWidth={2.5}
        strokeLinejoin="round"
      />
      {RUNGS.map(([x, y, level]) => (
        <circle
          key={level}
          cx={x}
          cy={y}
          r={5.5}
          fill={sample(level / 10)}
          stroke="var(--paper)"
          strokeWidth={1.4}
        />
      ))}
    </svg>
  )
}

const TAPE = [
  { seq: 4, kind: "text", body: "41.2s.", cl: true, op: false },
  { seq: 5, kind: "end", body: "turn complete", cl: true, op: false },
  { seq: 6, kind: "config", body: "intelligence(9)", cl: true, op: false },
  { seq: 7, kind: "switch", body: "moves to openai", cl: true, op: true },
  { seq: 8, kind: "start", body: "Why is it slow?", cl: false, op: true },
  { seq: 9, kind: "text", body: "Two cold starts.", cl: false, op: true },
]

function MiniTape() {
  return (
    <div className="mini-tape">
      {TAPE.map((row) => (
        <div key={row.seq} className="mini-row">
          <span className="mini-seq">{row.seq}</span>
          <span className="mini-cells">
            <i className={row.cl ? "on" : undefined} style={{ ["--tone" as string]: "var(--claude-code-cli)" }} />
            <i className={row.op ? "on" : undefined} style={{ ["--tone" as string]: "var(--codex-app-server)" }} />
          </span>
          <span className="mini-kind">{row.kind}</span>
          <span className="mini-body">{row.body}</span>
        </div>
      ))}
    </div>
  )
}

function MiniCode() {
  return (
    <pre className="mini-code"><Tokens>{`from omni import Inference

chat = Inference.load_or_create_session("triage")
chat.intelligence(7)
chat.start()
chat.send("what changed today?")`}</Tokens></pre>
  )
}

function MiniSeam() {
  return (
    <div className="mini-seam">
      <span className="mini-tag">turn 4</span>
      <div className="mini-bar">
        <i style={{ width: "22%" }}>thinking</i>
        <i className="tool" style={{ width: "40%" }}>Bash</i>
        <i style={{ width: "38%" }}>text</i>
        <s />
      </div>
      <span className="mini-pin">intelligence(9) — held</span>
      <span className="mini-tag">turn 5</span>
      <div className="mini-bar next">
        <i style={{ width: "100%" }}>runs on the new model</i>
      </div>
    </div>
  )
}

function MiniPipe() {
  return (
    <ol className="mini-pipe">
      {[
        ["models.json", "one file, one commit"],
        ["/choose.json?key=code", "a word in, a command out"],
        ["codex app-server …", "two strings, handed over"],
      ].map(([at, what], i) => (
        <li key={at}>
          <b style={{ ["--tone" as string]: sample(i / 2) }} />
          <code>{at}</code>
          <span>{what}</span>
        </li>
      ))}
    </ol>
  )
}

function MiniNots() {
  return (
    <ul className="mini-nots">
      {[
        "Carry reasoning across a switch.",
        "Define tools.",
        "Silence a project's own rules.",
        "Guess a benchmark number.",
        "Ship a model list.",
      ].map((n) => (
        <li key={n}>
          <b />
          {n}
        </li>
      ))}
    </ul>
  )
}

const ENTRIES = [
  {
    href: "#dial",
    wide: true,
    ask: "Which one is right for this?",
    how: "Five boards, and the dollars and seconds a task took. You weight the three, we answer.",
    art: <MiniFrontier />,
  },
  {
    href: "#log",
    ask: "What happens to the conversation?",
    how: "It lives in a file we keep, with one number that only goes up.",
    art: <MiniTape />,
  },
  {
    href: "#write",
    ask: "How much do I write?",
    how: "One shared session from Python, Rust or a terminal, and the logins already on your machine.",
    art: <MiniCode />,
  },
  {
    href: "#turn",
    wide: true,
    ask: "Can a model change halfway through my tool call?",
    how: "We wait for the turn to end. A message goes in where you sent it.",
    art: <MiniSeam />,
  },
  {
    href: "#database",
    wide: true,
    ask: "Where does the model list live?",
    how: "A versioned catalog. Sign in to Omni admin, edit, and publish.",
    art: <MiniPipe />,
  },
  {
    href: "#refusals",
    ask: "What do you refuse?",
    how: "Five things, each with its reason.",
    art: <MiniNots />,
  },
]

export function Contents() {
  return (
    <nav className="contents enter" aria-label="What is on this page">
      {ENTRIES.map((entry) => (
        <a key={entry.href} href={entry.href} className={`tile${entry.wide ? " wide" : ""}`}>
          {/* The miniature is decorative and visually clipped, so reading it
              into the link name announced six lines of Python in full while
              showing two. */}
          <div className="tile-window" aria-hidden>
            {entry.art}
            <span className="tile-blur" aria-hidden />
            <span className="tile-fade" aria-hidden />
          </div>
          <h3>{entry.ask}</h3>
          <p>{entry.how}</p>
        </a>
      ))}
    </nav>
  )
}
