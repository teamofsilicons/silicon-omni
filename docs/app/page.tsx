import { Copy } from "./copy"
import { Footer } from "./footer"
import { dialFor } from "../lib/dial"
import { graph } from "../lib/graph"

const INSTALL = "pip install silicon-omni"

const EVENTS: [string, string][] = [
  ["START", "the message that opened this turn"],
  ["TEXT", "one finished assistant message"],
  ["THINKING", "the model is reasoning. never what it thought"],
  ["TOOL.CALL", "tool, args, id"],
  ["TOOL.RESULT", "tool, id, result, ok"],
  ["END", "the turn is over"],
  ["INJECTED", "a message that landed mid-turn"],
  ["ERROR", "auth · limit · unavailable · crash"],
  ["SWITCH_PROVIDER", "the conversation moved"],
  ["NEW_SESSION", "a provider opened one of its own"],
]

export default async function Home() {
  const { models } = await graph()
  const rungs = dialFor(models, ["claude", "openai", "google"])

  return (
    <>
      <div className="hero">
        <h1 className="hero-brand">
          silicon
          <br />
          omni.
        </h1>
        <div className="hero-sub">
          <div className="hero-tagline">
            one interface for claude code,
            <br />
            codex and antigravity
            <br />
            <br />
            by{" "}
            <a className="strong" href="https://unlikefraction.com" target="_blank" rel="noreferrer">
              unlikefraction
            </a>
          </div>
          <nav className="hero-nav">
            <a href="#move">switching</a>
            <a href="#dial">the dial</a>
            <a href="#events">events</a>
            <a href="/reference">reference</a>
            <a href="/dial">edit the dial</a>
          </nav>
        </div>
      </div>

      <div className="install-bar">
        <code>
          <span className="dollar">$ </span>
          {INSTALL}
        </code>
        <Copy text={INSTALL} />
      </div>

      <section className="section">
        <h2>
          three clis.
          <br />
          one conversation.
        </h2>
        <p className="lede">
          Every vendor ships a good agentic CLI and a subscription that makes it cheap to run.
          None of them talk to each other. Pick one and you are married to its models, its rate
          limits and its outages.
        </p>
        <p>
          omni owns the conversation instead. Claude Code, Codex and Antigravity become
          interchangeable engines underneath a single Python object, driven by the subscriptions
          you already pay for rather than API keys. Zero runtime dependencies. You bring the CLIs.
        </p>

        <div className="terminal">
          <span className="dim"># the whole surface</span>
          {"\n"}
          from omni import Inference, Event{"\n\n"}
          chat = Inference.load_or_create_session(<span className="accent">&quot;nightly-triage&quot;</span>){"\n"}
          chat.intelligence(<span className="accent">7</span>){"\n\n"}
          <span className="dim">@chat.on_event</span>
          {"\n"}
          def handle(event):{"\n"}
          {"    "}if event.type == Event.TOOL.CALL:{"\n"}
          {"        "}print(event.tool, event.args){"\n\n"}
          chat.start(){"\n"}
          chat.send(<span className="accent">&quot;what changed in this repo today?&quot;</span>)
        </div>

        <div className="cards">
          <div className="card">
            <div className="num">claude code</div>
            <h4>Flags do the work</h4>
            <p>
              Subagents, MCP and memory files switch off from the command line. Seeding is a file
              write. Model changes go over the stdin control channel, so re-tuning never restarts
              anything.
            </p>
          </div>
          <div className="card">
            <div className="num">codex</div>
            <h4>App server, not exec</h4>
            <p>
              Runs against a CODEX_HOME of its own holding two things: a link to your real
              auth.json and a near-empty config. History arrives by thread/inject_items.
            </p>
          </div>
          <div className="card">
            <div className="num">antigravity</div>
            <h4>The awkward one</h4>
            <p>
              No flag for MCP, none for subagents, and no way to seed. omni folds prior
              conversation into the front of the next message, so catching up costs one turn
              rather than two.
            </p>
          </div>
        </div>
      </section>

      <section className="section" id="move">
        <h2>
          the conversation
          <br />
          moves.
        </h2>
        <p>
          Raise the dial mid-run and the next turn can land on a different vendor&apos;s model,
          with everything that came before already in its head. Coming back is cheaper still: the
          provider resumes its own session and is told only what it missed.
        </p>
        <p>
          This is a real run, not a mock. Three facts, three vendors, one session id.
        </p>

        <div className="terminal">
          <span className="dim">--- claude @ level 5 (claude-sonnet-5) ---</span>
          {"\n"}
          Remember: fact one is <span className="accent">RUBY-1</span>. Reply with exactly: OK{"\n"}
          <span className="green">✓ </span>OK{"\n\n"}
          <span className="dim">--- openai @ level 1 (gpt-5.4-mini) ---</span>
          {"\n"}
          <span className="accent">switch_provider</span>{"  "}claude → openai{"\n"}
          <span className="accent">new_session</span>
          {"     "}01a01d11-5ca8-7d02{"\n"}
          Remember: fact two is <span className="accent">JADE-2</span>. Reply with exactly: OK{"\n"}
          <span className="green">✓ </span>OK{"\n\n"}
          <span className="dim">--- google @ level 0 (gemini-3.5-flash-low) ---</span>
          {"\n"}
          <span className="accent">switch_provider</span>{"  "}openai → google{"\n"}
          Remember: fact three is <span className="accent">ONYX-3</span>. Reply with exactly: OK{"\n"}
          <span className="green">✓ </span>OK{"\n\n"}
          <span className="dim">--- claude @ level 5 ---</span>
          {"\n"}
          <span className="accent">switch_provider</span>{"  "}google → claude{"\n"}
          List all three facts, comma separated, values only.{"\n"}
          <span className="green">✓ </span>
          <span className="accent">RUBY-1, JADE-2, ONYX-3</span>
          {"\n\n"}
          <span className="dim">3 switches · 3 native sessions · 0 errors</span>
        </div>

        <h3>What actually crosses</h3>
        <p>
          <code className="inline">~/.omni/sessions/&#123;id&#125;.jsonl</code> is the source of
          truth, and it is the event log — the same objects your handlers see, appended in order.
          Alongside it, omni remembers each provider&apos;s own session and how far up the log it
          has already seen, so arriving somewhere replays only the part it missed.
        </p>
        <p>
          Providers do not share tools, so a tool the destination does not have is rendered as
          text that reads as what happened. The structured original stays in the log, which is why
          going back to Gemini replays Gemini&apos;s own session and the brackets never happened.
          Preserved, not lossy.
        </p>
        <div className="terminal">
          [GoogleSearch: <span className="accent">&quot;kite festivals&quot;</span>]{"\n"}
          [GoogleSearch result: 12 results …]
        </div>
        <p>
          Reasoning is the one thing that never travels. It is signed or encrypted per vendor and
          cannot be replayed anywhere else, so omni records that the model thought and throws the
          content away — including out of your logs.
        </p>
      </section>

      <section className="section" id="dial">
        <h2>
          one number.
          <br />
          not a model picker.
        </h2>
        <p>
          Every model the three CLIs can run is plotted by its{" "}
          <a href="https://artificialanalysis.ai/evaluations/gdpval-aa" target="_blank" rel="noreferrer">
            GDPval-AA v2
          </a>{" "}
          Elo — blind pairwise judging of real economically valuable work, anchored so a human
          expert scores 1000 — against the dollars it measurably cost to earn that score.
        </p>
        <p>
          Only the left edge of that graph becomes a dial. A model earns a level if nothing else
          is both better <em>and</em> cheaper. Level 10 is the top of the edge, and the dial walks
          down-left from there, so a step down is always a real saving and never a sideways move.
        </p>

        <div className="grid" style={{ gridTemplateColumns: "auto auto auto 1fr" }}>
          <div className="gh">level</div>
          <div className="gh">elo</div>
          <div className="gh">$/task</div>
          <div className="gh">model</div>
          {Array.from({ length: 11 }, (_, i) => 10 - i).map((level) => {
            const rung = rungs[String(level)]
            if (!rung) return null
            return (
              <div key={level} style={{ display: "contents" }}>
                <div className="gc strong">{level}</div>
                <div className="gc">{rung.score}</div>
                <div className="gc quiet">{rung.price}</div>
                <div className="gc">
                  {rung.model} {rung.effort}
                </div>
              </div>
            )
          })}
        </div>

        <p>
          Level 4 is worth staring at. GPT-5.6 Luna at max effort scores within 15% of the top of
          the board for 66 times less money, which is why everything between it and Opus 5 falls
          off the edge entirely.
        </p>
        <p>
          There is one dial per set of providers, because losing a vendor puts models back on the
          dial that another vendor&apos;s were shadowing. omni asks this site for the dial matching
          the providers it has and caches it for an hour.{" "}
          <a href="/dial">The dial is editable</a> — add a model, and every install picks it up on
          its next refresh.
        </p>
        <div className="row">
          <a className="btn-dark" href="/dial">
            edit the dial
          </a>
          <a className="btn-outline" href="/api/intelligence?providers=claude+google+openai">
            see the json
          </a>
        </div>
      </section>

      <section className="section" id="events">
        <h2>one vocabulary.</h2>
        <p>
          Everything omni has to say arrives as one <code className="inline">Event</code>. The same
          objects go to your handlers, to your logs, and onto disk — so the session file{" "}
          <em>is</em> the event log, and there is never a second schema to learn.
        </p>

        <div className="grid" style={{ gridTemplateColumns: "auto 1fr" }}>
          <div className="gh">type</div>
          <div className="gh">carries</div>
          {EVENTS.map(([name, carries]) => (
            <div key={name} style={{ display: "contents" }}>
              <div className="gc strong">{name}</div>
              <div className="gc quiet">{carries}</div>
            </div>
          ))}
        </div>

        <p>
          Handlers run on one thread, in the order things actually happened. A handler that raises
          is reported and stepped over — it cannot take the run down with it.
        </p>
      </section>

      <section className="section">
        <h2>
          nothing changes
          <br />
          mid turn.
        </h2>
        <p>
          This is the rule the whole design hangs off. Intelligence, providers, prompts and session
          swaps are recorded the moment you ask for them and applied at the next turn boundary,
          once the running tool has finished. A model never changes underneath itself.
        </p>
        <div className="terminal">
          chat.intelligence(<span className="accent">9</span>){"              "}
          <span className="dim"># noted now, applied at the boundary</span>
          {"\n"}
          chat.active_inference_providers([<span className="accent">&quot;claude&quot;</span>,{" "}
          <span className="accent">&quot;openai&quot;</span>]){"\n"}
          chat.system_prompt(...){"\n"}
          chat.disable_subagents(){"\n"}
          chat.disable_mcp()
        </div>
        <p>
          A message sent while a turn is running is <em>injected</em> rather than queued behind it:
          the provider picks it up at the next safe point. Either way{" "}
          <code className="inline">send</code> returns immediately and is safe from any thread, so
          whether you drive omni from a while loop, asyncio or a message bus is entirely your
          business.
        </p>
      </section>

      <section className="section">
        <h2>what it will not do.</h2>
        <p>
          A short list, because the things a tool refuses to do tell you more than the things it
          claims.
        </p>
        <ul>
          <li>
            <strong>Carry reasoning across a switch.</strong> Signed or encrypted per vendor, so it
            cannot be replayed. The new model reasons from scratch.
          </li>
          <li>
            <strong>Define tools.</strong> omni observes whatever the CLI exposes and never
            installs, renames or invents one.
          </li>
          <li>
            <strong>Hold more than one account per provider.</strong>
          </li>
          <li>
            <strong>Pretend Antigravity can be muzzled.</strong>{" "}
            <code className="inline">disable_subagents()</code> and{" "}
            <code className="inline">disable_mcp()</code> have no equivalent there, so omni logs
            that it ignored you rather than quietly doing nothing.
          </li>
          <li>
            <strong>Guess a benchmark number.</strong> A model GDPval has not scored or costed is
            left off the dial rather than estimated onto it.
          </li>
        </ul>
        <div className="row">
          <a className="btn-dark" href="/reference">
            read the reference
          </a>
          <a className="btn-outline" href="/dial">
            edit the dial
          </a>
        </div>
      </section>

      <Footer />
    </>
  )
}
