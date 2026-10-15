import { Logo, type Vendor } from "./logos"

/**
 * Three grammars, one gauge.
 *
 * The tracks are drawn at three different tie spacings on purpose. Every
 * vendor ships a good CLI and no two of them speak the same way — one takes
 * flags and JSON lines on stdin, one takes JSON-RPC over a socket, one takes
 * neither and has to be handed its history inside the next message. It is a
 * break of gauge, and the only honest way to draw it is to draw it as one.
 */

const RAILS = [
  {
    id: "claude-code-cli",
    label: "claude-code-cli",
    gauge: 6,
    holds: "claude -p --output-format stream-json --input-format stream-json",
    grammar: "one long-lived process. a turn is one JSON line on stdin",
    catch:
      "Model and effort go down the same stdin control channel the turns do, so the process stays up while the dial moves.",
  },
  {
    id: "codex-app-server",
    label: "codex-app-server",
    gauge: 13,
    holds: "codex app-server --stdio",
    grammar: "JSON-RPC over newline-delimited JSON. turn/start, or turn/steer into a turn already running",
    catch:
      "CODEX_HOME is pointed at a jail. Inside it: a symlink to your real auth.json, and one near-empty config. That is the whole of what it loads.",
  },
  {
    id: "antigravity-cli",
    label: "antigravity-cli",
    gauge: 24,
    holds: 'agy --output-format stream-json --input-format stream-json --print ""',
    grammar: 'the empty --print goes last, because it takes a value',
    catch:
      "agy resumes its own conversation by id, so consecutive turns cost nothing extra. Only what it missed while another provider answered rides in the front of the next message.",
  },
]

export function Grammars() {
  return (
    <div className="rails-fig">
      {RAILS.map((rail) => (
        <div key={rail.id} className="rail" style={{ ["--tone" as string]: `var(--${rail.id})`, ["--gauge" as string]: `${rail.gauge}px` }}>
          <div className="rail-name">
            <Logo of={rail.id as Vendor} size={15} />
            <b>{rail.label}</b>
            <span className="num">{rail.grammar}</span>
          </div>
          <div className="rail-track" aria-hidden />
          <code className="rail-holds">{rail.holds}</code>
          <p className="rail-catch">{rail.catch}</p>
        </div>
      ))}
      <div className="rail-join">
        <span className="rail-join-track" aria-hidden />
        <code className="rail-join-code">chat.send(&quot;what changed in this repo today?&quot;)</code>
      </div>
    </div>
  )
}
