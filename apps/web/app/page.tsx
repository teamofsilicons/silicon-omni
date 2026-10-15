import { ArrowRight, ChartScatter, Prohibit } from "@phosphor-icons/react/dist/ssr"

import { Boundary } from "./boundary"
import { Chapter, Plate } from "./chapter"
import { Contents } from "./contents"
import { Code } from "./code"
import { Command } from "./command"
import { Picker } from "./picker"
import { Dither } from "./dither"
import { Field } from "./field"
import { Grammars } from "./grammars"
import { Mark } from "./mark"
import { Logo, type Vendor } from "./logos"
import { Pipeline } from "./pipeline"
import { Relay } from "./relay"
import { Tape } from "./tape"
import { byKey, edge, plottable } from "../lib/choose"
import { graph } from "../lib/graph"
import { dial as levelColour, on as inkOn } from "../lib/ramp"

const QUICKSTART = `from omni import Inference, Event

chat = Inference.load_or_create_session("triage")
chat.intelligence(7)

@chat.on_event
def handle(event):
    if event.type == Event.TEXT:
        print(event.text)

chat.start()
chat.send("what changed in this repo today?")`

const CHAPTERS = 8

export default async function Landing() {
  const { models, benches, keys } = await graph()
  const names = Object.keys(benches)
  const words = Object.keys(keys)

  /* One worked example, computed rather than typed, so the prose under it can
     never contradict the control above it. */
  const SHOWN = names.includes("gdpval-aa") ? "gdpval-aa" : names[0]
  const WORD = words.includes("code") ? "code" : words[0]
  const ALL = ["claude-code-cli", "codex-app-server", "antigravity-cli"]
  const example = byKey(keys, WORD, ALL)
  const best = example?.pick ?? null

  /* The two providers the log figure runs on, read out of a shortlist rather
     than typed into the figure: the first choice, and the next vendor down. */
  const other = example?.shortlist.find((p) => p.provider !== best?.provider) ?? null
  const asHeld = (m: { provider: string; model: string; effort: string } | null, level: number) =>
    m ? { level, provider: m.provider, model: m.model, effort: m.effort } : { level, provider: "claude-code-cli", model: "-", effort: "" }

  /* What the board actually looks like today, counted rather than asserted. */
  const distinct = new Set(models.map((m) => m.model)).size
  const walk = edge(plottable(models.filter(m => m.status !== "legacy" && m.status !== "restricted"), SHOWN), SHOWN)

  /* Numbers where a landing page would put adjectives. Counted out of the
     database at request time, so they cannot go stale against the board. */
  const TALLY: [string, string][] = [
    [String(models.length), `settings we can run — ${distinct} models at the efforts each takes`],
    [String(words.length), "words that answer for you, chosen by hand"],
    ["3", "command lines, one session"],
    ["0", "Python runtime dependencies"],
    ["1", "file holds the conversation, whoever answers it"],
  ]

  return (
    <>
      {/* Two bands, not an overlay.
          The type used to sit on top of the chart, in a hole the chart masked
          out of itself. That only works while the sentence stays the height it
          was when the hole was cut — and the sentence is copy, so it doesn't.
          A headline's height and a frontier's shape are independent variables;
          asking them to miss each other was a bet, and it lost. */}
      <section className="hero">
        <Dither shape="radial" cells={210} />
        <div className="shell hero-shell">
          <div className="hero-grid">
            <h1 className="rise">
              We keep the conversation.
              <br />
              The three of them take&nbsp;turns.
            </h1>
            <div className="hero-side">
              <p className="lede rise" style={{ ["--i" as string]: 1 }}>
                We drive Claude Code, Codex and Antigravity through one session — from Python,
                Rust, or a terminal — on the subscriptions you already have. Say the work is worth
                more and the same chat finishes on another company&apos;s model, knowing
                everything said before it.
              </p>
              <div className="row rise" style={{ ["--i" as string]: 2 }}>
                <Command text="pip install silicon-omni" />
                <a className="btn ghost" href="/docs">
                  documentation
                  <ArrowRight size={14} weight="bold" />
                </a>
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="plot">
        <Field models={models} benches={benches} />
      </section>

      <div className="shell rails">
        <div className="tally enter">
          {TALLY.map(([n, what]) => (
            <div key={what}>
              <b>{n}</b>
              <span>{what}</span>
            </div>
          ))}
        </div>

        {/* The admission, and it goes above everything it complicates. Lemon
            convicts the car in the headline and sells more cars for it; the
            equivalent here is that the thing which makes omni cheap is also
            the thing nobody can indemnify you against. */}

        <Contents />

        <Chapter
          n={1}
          of={CHAPTERS}
          id="gauge"
          name="the break of gauge"
          title="Each of the three keeps its own memory."
          argue={
            <>
              <p>
                Pick a good one and you have picked its models, its rate limits and its bad
                afternoons with it. Leaving later costs you the integration and the conversation you
                were in the middle of. The second one is the expensive part, and it is the one
                nobody quotes you for.
              </p>
              <p>
                We keep the conversation in a file of our own. The three become engines, and you can
                change an engine while it runs.
              </p>
            </>
          }
        >
          {/* Brunel really did promise the Bristol company that transshipment
              would be trivial — the substance is well attested — but the exact
              wording traces only to MacDermot's History of the GWR at second
              hand, and archive.org and HathiTrust full-text are both closed to
              us. So it is paraphrased. Do not put the quotation marks back
              without reading vol. I pt. 1 p. 211 yourself. */}
          <Plate
            name="break-of-gauge"
            eager
            alt="Porters transferring freight between broad-gauge and standard-gauge trains at Gloucester, 1846"
            why={
              <>
                Seven foot and a quarter inch on one side of the shed, four foot eight and a half on
                the other, and a hundred men in between moving every crate across by hand. Brunel
                had assured the Bristol company that shifting a whole load from one company&apos;s
                waggon to the other&apos;s would need only a very simple arrangement. The engraving
                is the arrangement.
              </>
            }
          />
          <Grammars />
          <div className="spread lean">
            <p className="set">
              Three grammars, drawn here at three gauges. One takes flags and JSON lines on stdin.
              One takes JSON-RPC over a socket. One takes its history inside the front of the next
              message. Once one is active, we keep its process and native conversation open across
              consecutive turns. A switch may cost an activation; the arriving provider gets the
              same story in the dialect it accepts.
            </p>
            <Plate
              name="sea-floor"
              aside
              alt="Engraving: masons laying a wall on the sea floor, air hoses running up through the waterline to a crew working in the open above"
              why={
                <>
                  Every man down there breathes through his own line. The wall goes up all the same.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={2}
          of={CHAPTERS}
          id="write"
          name="what you write"
          title="One session, and the logins already on your machine."
          argue={
            <>
              <p>
                We offer the providers whose command line is installed <em>and</em> signed in.
                That is the list you get.
              </p>
              <p>
                Python, Rust and terminal clients attach to the same daemon. <code>send</code> from
                any client, <code>intelligence</code> to change what answers, <code>detach</code> to
                leave it warm, and <code>stop</code> to end the shared session.
              </p>
            </>
          }
        >
          <div className="spread lean">
            <Code>{QUICKSTART}</Code>
            <Plate
              name="movable-type"
              aside
              alt="A diagram naming every part of a single piece of metal type"
              why={
                <>
                  Ojo, cuerpo, cran, línea: an apprentice was expected to know every part by name
                  before he was allowed to set a word. Everything ever printed is this object, a few
                  thousand times over, in an order somebody chose.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={3}
          of={CHAPTERS}
          id="log"
          name="one integer"
          title="Every line gets a number, and the number only goes up."
          argue={
            <>
              <p>
                The session is a log we append to. Every line carries a number, and the number
                runs on through every provider the conversation has passed through. A second,
                smaller file holds how far up that log each one has read.
              </p>
              <p>
                Two files, and the rest follows. A provider arriving hears what it missed. A
                provider coming back resumes <em>its own</em> session and hears the rest.
              </p>
            </>
          }
        >
          <Tape high={asHeld(best, 10)} low={asHeld(other, 4)} />
          <div className="spread">
            <div>
              <p className="set">
                Watch the second column fill. On the way out, the arriving provider is handed eight
              lines. Coming back, the first one is handed seven of the fourteen the file now holds,
              because the first six are already inside its own session and it is resuming that.
            </p>
            <p className="set">
              Benedict gave the kitchen a week at a time, and gave the changeover to both men at
              once: the server going off washes up and returns the utensils to the cellarer, who
              hands them to the server coming on,{" "}
              <a className="link" href="https://www.gutenberg.org/files/50040/50040-h/50040-h.htm#Chapter_35">
                in order that he may know what he gives out and what he receives back
              </a>
              . The second file is the cellarer&apos;s list.
            </p>
              <Plate
                name="punched-tape"
                aside
                alt="A Hollerith punched card from 1895, its columns printed with the digits nought to nine"
                why={
                  <>
                    Each column is printed nought to nine, and the hole says which. A number you can
                    drop on the floor and still put back in order.
                  </>
                }
              />
            </div>
            <Plate
              name="ships-log"
              aside
              alt="A page from the log of HMS Amphitrite for 13 August 1914, hours ruled down the margin"
              why={
                <>
                  The hours are ruled down the margin before the day happens. An hour with a blank
                  against it still counts as an hour.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={4}
          of={CHAPTERS}
          id="switch"
          name="the relay"
          title="We tell the next one what it missed."
          argue={
            <p>
              A real run, transcribed. One fact each to three companies&apos; models, then the
              question put back to the first. Each of them heard its own line once.
            </p>
          }
        >
          <Relay />

          <Plate
            name="switchboard"
            alt="Operators at a Bell System international switchboard, patching calls by hand"
            why={
              <>
                LONDON. PARIS. BERMUDA. BUENOS AIRES. SHIP-TO-SHORE. For eighty years the network
                between two exchanges was a woman holding both ends of it, and she is the part the
                diagrams leave out.
              </>
            }
          />

          <div className="spread lean">
            <p className="set">
              A tool that belongs to somebody else arrives as text that reads as what happened —{" "}
              <code>[GoogleSearch: &quot;kite festivals&quot;]</code>. The structured original stays
              in the log, so going back to Gemini resumes Gemini&apos;s own session, which holds it.
            </p>
            <p className="set">
              When a bob is called, the bells in fourth and fifth place ring on{" "}
              <a className="link" href="https://archive.org/details/changeringingan00troygoog">
                as if no calling had been made
              </a>
              . Coming back, so does the provider.
            </p>
            <Plate
              name="wax-seal"
              aside
              alt="A 1176 charter of Philip of Alsace with his wax seal hanging from a parchment tag"
              why={
                <>
                  The words copy. The seal stays where it was pressed. Reasoning is the seal, which
                  is why we record that the model thought and keep the thought out of the file.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={5}
          of={CHAPTERS}
          id="dial"
          name="the ask"
          title="Say what the work is. We say what runs it."
          argue={
            <>
              <p>
                One word for the kind of work, and the answer comes back — a
                shortlist in the configured priority order. The first eligible model on a
                signed-in provider wins; Argon falls back to Flash when access is not confirmed.
              </p>
              <p>
                If you would rather say a number, say a number. If you already know the model, name
                it and we will pass it through untouched.
              </p>
            </>
          }
        >
          <Picker keys={keys} />

          <div className="spread lean">
            <div>
              <p className="set">
                We tried to work this out arithmetically first — score, cost and time, normalised
                and weighted, one number out the other end. It kept producing answers that were
                defensible and wrong: three models within one percent of each other and a hundred
                Elo apart, and a winner that moved when a constant did. Ranking is not judgement,
                so the judgement is written down instead.
              </p>
              <p className="set">
                Ask for <strong>{WORD}</strong> and today you get{" "}
                <strong>
                  {best?.model}
                  {best?.effort ? ` ${best.effort}` : ""}
                </strong>{" "}
                on {best?.provider}. Sign out of {best?.provider} and it walks one place down the
                list, which is the whole cost of losing a vendor, said plainly.
              </p>
              <p className="set">
                Two of the three run hot on request. Codex takes{" "}
                <code>service_tier</code>; Claude Code has a fast mode on its largest models.
                Antigravity has no such thing, so asking for it there is ignored rather than
                refused.
              </p>
              <p className="set">
                The default number follows current recommendations: cost, speed, general work,
                coding, then research as you turn it up. An explicit benchmark instead uses
                measured cost and quality.{" "}
                <a className="link" href="/models">
                  See the whole board
                </a>{" "}
                and turn a provider off.
              </p>
              <p className="set">
                Benchmark rankings only include current models with comparable measurements.
                New models can enter curated recommendations immediately; missing scores remain
                unmeasured. Each board links its source and methodology.
              </p>
              <p className="set">
                A bee hunter gets his straight line out of the eighth or tenth flight. The first few
                point somewhere else.
              </p>
            </div>
          </div>
        </Chapter>

        <Chapter
          n={6}
          of={CHAPTERS}
          id="turn"
          name="the seam"
          title="We wait for the turn to end."
          argue={
            <>
              <p>
                Intelligence, providers, prompts and session swaps are written down the moment you
                ask for them, and applied at the next turn boundary, once the running tool has
                finished.
              </p>
              <p>
                A message is the exception, and on purpose. Sent while a turn is running, it goes
                into that turn.
              </p>
            </>
          }
        >
          <blockquote className="pull">
            <p>
              If, at any time, the officer in charge of the navigational watch is to be relieved when
              a manoeuvre or other action to avoid any hazard is taking place, the relief of that
              officer shall be deferred until such action has been completed.
            </p>
            <cite className="num">
              STCW Code, Section A-VIII/2, Part 4-1, paragraph 23 · the Manila amendments, 2010 ·{" "}
              <a href="https://wwwcdn.imo.org/localresources/en/OurWork/HumanElement/Documents/34.pdf" rel="noreferrer">
                International Maritime Organization
              </a>
            </cite>
          </blockquote>

          <p className="set" style={{ maxWidth: "38rem" }}>
            A ship under pilotage separates two things that software usually confuses. The pilot has
            the <em>conduct</em> of the vessel; the master stays in{" "}
            <em>command</em>. The provider has conduct of the
            turn — it decides what to say and which tool to reach for. We have command: the
            log, the counter, and the moment at which anything is allowed to change.
          </p>

          <Boundary />
          <div className="spread lean">
            <Plate
              name="lock-gate"
              aside
              alt="Joseph Pennell's 1912 lithograph of the guard gate at Gatun Lock, standing closed in a dry chamber"
              why={
                <>
                  Drawn while the chamber was still dry, so the gate can be seen at rest. A ship
                  waits between two shut gates while the water finds its new level. Everyone who has
                  tried it another way has stopped.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={7}
          of={CHAPTERS}
          id="refusals"
          name="the refusals"
          title="Five things we refuse."
          argue={
            <p>Five, with the mechanism for each.</p>
          }
        >
          <div className="spread lean">
            <div className="nots">
              {[
                ["Carry reasoning across a switch.", "A vendor signs its reasoning, so it replays only where it was made. We record that the model thought, and drop the thought — out of your logs as well."],
                ["Define tools.", "We watch what the command line already exposes."],
                ["Silence a project's own rules.", "agy reads a GEMINI.md or an AGENTS.md out of the directory it runs in and offers no way to stop it. We say so on the turn rather than call the chat quiet."],
                ["Guess a benchmark number.", "A model a board has measured is a candidate on that board. One it has yet to measure stays on the list and out of the answer."],
                ["Ship a model list.", "A list inside a release goes stale. With the registry unreachable and the cache empty, we raise."],
              ].map(([what, why]) => (
                <div key={what} className="not">
                  <Prohibit size={17} weight="bold" className="not-icon" />
                  <div>
                    <h3>{what}</h3>
                    <p>{why}</p>
                  </div>
                </div>
              ))}
            </div>
            <Plate
              name="no-admittance"
              aside
              alt="An 1878 quarantine notice from the Cape Girardeau Board of Health, forbidding any vessel to land"
              why={
                <>
                  Ordered by the Board of Health: no steamboat to land freight or passengers within
                  five miles of the town. A notice is useful in exact proportion to how precisely it
                  names what it refuses.
                </>
              }
            />
          </div>
        </Chapter>

        <Chapter
          n={8}
          of={CHAPTERS}
          id="database"
          name="the database"
          title="One catalog. Updated as models arrive."
          aside={
            <p className="ch-aside set">
            It matters whose file it is. The shipowners kept one register of ships and the
            underwriters kept another: the Red Book and the Green Book, 1800 to 1833, grading the
            same hulls, each believed only by the party that had printed it. Both came to the verge
            of bankruptcy. Merged 1834, into{" "}
            <a className="link" href="https://www.lr.org/en/who-we-are/brief-history/">
              one body neither of them owned
            </a>
            . That is the arrangement that survived. A routing table maintained by any one of these
            three vendors would be the Red Book.
            </p>
          }
          argue={
            <>
              <p>
                We ask this site for a finished map from level to model, and hand two strings to a
                command line. Every model name on your machine arrived over the wire this morning.
              </p>
              <p>
                The map comes from a versioned catalog. Sign in to Omni admin with Silicon
                Accounts, add a model or change a recommendation, and publish. Every save records
                its author and keeps earlier versions.
              </p>
              <p>
                Model queries are public. Only approved administrators can change the catalog.
                Your clients pick up published recommendations automatically, and{" "}
                <code>OMNI_REGISTRY</code> lets you point at your own service.
              </p>
            </>
          }
        >
          <div className="spread lean">
            <Pipeline />
            <Plate
              name="stale-catalogue"
              aside
              alt="The Price Act broadside of 1777, fixing the price of every commodity in the town of Ipswich"
              why={
                <>
                  Every commodity in the town, priced by ordinance and set in display caps. It was
                  wrong by March, and any model list baked into a release is this broadside.
                </>
              }
            />
          </div>
        </Chapter>

        {/* A rest, rather than a gap. The mark alone on an empty band was a
            held breath with nothing in it; a flock is the same argument the
            whole page has been making, and it is nobody's design. */}
        <figure className="rest">
          <div className="rest-plate">
            <img
              src="/plates/murmuration.png"
              srcSet="/plates/murmuration.png 1x, /plates/murmuration@2x.png 2x"
              alt="A dense band of red-winged blackbirds along the horizon, its upper edge rolling"
              width={720}
              height={240}
              loading="lazy"
              decoding="async"
            />
          </div>
          <figcaption>
            <Mark h={13} />
            <span>
              Nobody drew the top edge of that flock, and it is the sharpest line in the photograph.
              Every frontier on this site is made the same way.
            </span>
          </figcaption>
        </figure>

        <section className="close enter">
          <h2 className="column">
            One <code>pip install</code>, and one number.
          </h2>
          <div className="row" style={{ marginTop: "1.8rem" }}>
            <a className="btn solid" href="/docs">
              documentation
              <ArrowRight size={14} weight="bold" />
            </a>
            <a className="btn ghost" href="/models">
              <ChartScatter size={15} weight="bold" />
              the models graph
            </a>
          </div>
        </section>
      </div>
    </>
  )
}
