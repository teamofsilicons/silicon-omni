import { Footer } from "../footer"
import { PROVIDERS } from "../../lib/providers"

export const metadata = {
  title: "reference | silicon omni",
  description: "Every method, event, file and endpoint in silicon omni.",
}

const CHAT: [string, string][] = [
  ["chat.start()", "Bring a provider up and begin. Returns immediately."],
  ["chat.send(text)", "Open a turn, or land inside the one already running."],
  ["chat.stop()", "Shut the provider down and release the session id."],
  ["chat.status", "idle before start, then busy / waiting, then stopped."],
  ["chat.idle", "Waiting with nothing queued. What a polling loop should check."],
  ["chat.intelligence(n)", "0-10 across every active provider. May change vendor."],
  ["chat.active_inference_providers([...])", "Narrow which providers omni may route to."],
  ["chat.system_prompt(text)", "Replace the provider's own session prompt."],
  ["chat.system_prompt_file(path)", "The same, from a file."],
  ["chat.append_system_prompt(text)", "Keep theirs and add to it."],
  ["chat.append_system_prompt_file(path)", "The same, from a file."],
  ["chat.disable_subagents()", "So only the workers you define get used."],
  ["chat.disable_mcp()", "No MCP servers, no external connectors."],
  ["chat.cwd(path)", "Where the provider runs its tools. Pinned to the session."],
  ["@chat.on_event", "Decorator. Every event, as it happens."],
  ["@chat.logs", "Decorator. Everything on_event sees, plus omni's bookkeeping."],
]

const EVENT_FIELDS: [string, string][] = [
  ["type", "which of the event types this is"],
  ["session", "the omni session id"],
  ["provider", "claude · openai · google"],
  ["model", "the model as the provider reported it"],
  ["text", "message content, for the types that carry it"],
  ["tool / id / args", "on TOOL.CALL"],
  ["result / ok", "on TOOL.RESULT"],
  ["kind / error", "on ERROR"],
  ["at / seq", "when it happened, and where in the session"],
  ["extra", "anything else the provider said"],
]

export default function Reference() {
  return (
    <>
      <div className="page-head">
        <div className="crumb">
          <a href="/">silicon omni</a> / reference
        </div>
        <h1>reference.</h1>
        <p>
          Everything omni exposes, what each provider does underneath, and the endpoint this site
          serves the dial from.
        </p>
        <div className="toc">
          <a href="#install">install</a>
          <a href="#inference">inference</a>
          <a href="#chat">chat</a>
          <a href="#events">events</a>
          <a href="#sessions">sessions</a>
          <a href="#auth">auth</a>
          <a href="#limits">limits</a>
          <a href="#providers">providers</a>
          <a href="#registry">registry</a>
        </div>
      </div>

      <section className="section" id="install">
        <h2>install.</h2>
        <div className="terminal">
          <span className="dim">$ </span>pip install silicon-omni
        </div>
        <p>
          Zero runtime dependencies. You bring the CLIs, and omni only offers the ones that are
          both installed and signed in.
        </p>
        <div className="grid" style={{ gridTemplateColumns: "auto auto 1fr" }}>
          <div className="gh">provider</div>
          <div className="gh">cli</div>
          <div className="gh">how omni drives it</div>
          {PROVIDERS.map((p) => (
            <div key={p.id} style={{ display: "contents" }}>
              <div className="gc strong">{p.id}</div>
              <div className="gc">{p.label}</div>
              <div className="gc quiet">{p.cli}</div>
            </div>
          ))}
        </div>
        <div className="terminal">
          Inference.get_available_providers(){"\n"}
          <span className="dim"># ['claude', 'google', 'openai']</span>
        </div>
      </section>

      <section className="section" id="inference">
        <h2>inference.</h2>
        <p>The front door. The only object you need to import, besides the events.</p>
        <div className="terminal">
          from omni import Inference, Event{"\n\n"}
          PROVIDERS = Inference.get_available_providers(){"\n"}
          chat = Inference.load_or_create_session(<span className="accent">&quot;session-id&quot;</span>, PROVIDERS){"\n\n"}
          Inference.claude.auth_status{"      "}
          <span className="dim"># authenticated | unauthenticated</span>
          {"\n"}
          Inference.claude.installed{"        "}
          <span className="dim"># is the cli even here</span>
          {"\n"}
          Inference.claude.limits{"           "}
          <span className="dim"># quota, without spending a token</span>
        </div>
        <p>
          <code className="inline">Inference.claude</code>,{" "}
          <code className="inline">Inference.openai</code> and{" "}
          <code className="inline">Inference.google</code> are the three account handles. They are
          resolved lazily, so touching one never imports the other two.
        </p>
        <p>
          One live chat per session id. A second attempt raises{" "}
          <code className="inline">SessionBusy</code>; a lock whose owner has died is reclaimed, so
          a crash never wedges a session shut.
        </p>
      </section>

      <section className="section" id="chat">
        <h2>chat.</h2>
        <div className="grid" style={{ gridTemplateColumns: "auto 1fr" }}>
          <div className="gh">call</div>
          <div className="gh">what it does</div>
          {CHAT.map(([call, what]) => (
            <div key={call} style={{ display: "contents" }}>
              <div className="gc strong">{call}</div>
              <div className="gc quiet">{what}</div>
            </div>
          ))}
        </div>

        <h3>Nothing changes mid turn</h3>
        <p>
          Every setter above is recorded when you call it and applied at the next turn boundary.
          Calling one again overwrites the last value. That is also how a new session is started:
          call <code className="inline">load_or_create_session</code> again.
        </p>

        <h3>Driving it</h3>
        <p>
          <code className="inline">send</code> never blocks and is safe from any thread, including
          from inside an event handler, so the loop is yours to choose.
        </p>
        <div className="terminal">
          <span className="dim"># a plain loop</span>
          {"\n"}
          chat.start(){"\n"}
          while chat.status in (<span className="accent">&quot;busy&quot;</span>,{" "}
          <span className="accent">&quot;waiting&quot;</span>):{"\n"}
          {"    "}message = fetch_new_messages(){"\n"}
          {"    "}if message:{"\n"}
          {"        "}chat.send(message){"\n"}
          {"    "}else:{"\n"}
          {"        "}time.sleep(<span className="accent">0.2</span>){"\n"}
          {"    "}if should_stop() and last_event == Event.END:{"\n"}
          {"        "}chat.stop()
        </div>
        <div className="terminal">
          <span className="dim"># or a subscription</span>
          {"\n"}
          async def on_msg(m):{"\n"}
          {"    "}chat.send(m.data.decode()){"     "}
          <span className="dim"># opens a turn, or lands mid-flight</span>
          {"\n"}
          {"    "}await m.ack(){"\n\n"}
          await nc.subscribe(<span className="accent">&quot;agent.msgs&quot;</span>, cb=on_msg)
        </div>
      </section>

      <section className="section" id="events">
        <h2>events.</h2>
        <p>
          One dataclass for everything. Class attributes are the types, instance fields are the
          payload, and which fields are filled depends on the type.
        </p>
        <div className="grid" style={{ gridTemplateColumns: "auto 1fr" }}>
          <div className="gh">field</div>
          <div className="gh">meaning</div>
          {EVENT_FIELDS.map(([field, meaning]) => (
            <div key={field} style={{ display: "contents" }}>
              <div className="gc strong">{field}</div>
              <div className="gc quiet">{meaning}</div>
            </div>
          ))}
        </div>
        <p>
          <code className="inline">Event.ERROR</code> carries a{" "}
          <code className="inline">kind</code>: <code className="inline">auth</code>,{" "}
          <code className="inline">limit</code>, <code className="inline">unavailable</code> or{" "}
          <code className="inline">crash</code> for the model and its CLI, plus{" "}
          <code className="inline">stderr</code> for CLI chatter,{" "}
          <code className="inline">omni</code> when the engine itself failed, and{" "}
          <code className="inline">handler</code> when one of your callbacks raised.
        </p>
        <h3>Logging</h3>
        <div className="terminal">
          <span className="dim">@chat.logs</span>
          {"\n"}
          def log(event):{"\n"}
          {"    "}write_somewhere(event.to_dict())
        </div>
        <p>
          Everything <code className="inline">on_event</code> sees plus omni&apos;s own
          bookkeeping: every launch, model change, provider switch, new session, message in, tool
          call, error and stop. All of it the same type, so it is parsable without a second schema
          — and it is already on disk in the session file.
        </p>
      </section>

      <section className="section" id="sessions">
        <h2>sessions.</h2>
        <div className="grid" style={{ gridTemplateColumns: "auto 1fr" }}>
          <div className="gh">path</div>
          <div className="gh">what it holds</div>
          <div className="gc strong">~/.omni/sessions/&#123;id&#125;.jsonl</div>
          <div className="gc quiet">the conversation, as an append-only event log</div>
          <div className="gc strong">~/.omni/sessions/&#123;id&#125;.meta.json</div>
          <div className="gc quiet">each provider&apos;s own session, and how far it is synced</div>
          <div className="gc strong">~/.omni/sessions/&#123;id&#125;.lock</div>
          <div className="gc quiet">the owning pid, reclaimed if that pid is gone</div>
          <div className="gc strong">~/.omni/jails/&#123;id&#125;/codex</div>
          <div className="gc quiet">codex&apos;s stripped CODEX_HOME</div>
          <div className="gc strong">~/.omni/cache/intelligence.json</div>
          <div className="gc quiet">the dial, per provider set, for an hour</div>
        </div>
        <h3>How a switch works</h3>
        <ul>
          <li>
            <strong>Continuing on the same provider</strong> uses its native resume. omni&apos;s log
            is not read at all.
          </li>
          <li>
            <strong>Arriving somewhere new</strong> replays only the part that provider missed.
          </li>
          <li>
            <strong>Coming back</strong> resumes its own session and tops it up with what happened
            while it was away.
          </li>
          <li>
            <strong>A provider that lost its session</strong> reports an id omni did not ask for,
            and gets told the whole story from the top.
          </li>
        </ul>
      </section>

      <section className="section" id="auth">
        <h2>auth.</h2>
        <div className="terminal">
          Inference.claude.auth_status{"\n"}
          print(Inference.claude.start_auth()){"      "}
          <span className="dim"># the url to open</span>
          {"\n"}
          Inference.claude.finish_auth(<span className="accent">&quot;code-or-redirect-url&quot;</span>)
        </div>
        <p>
          omni drives each CLI&apos;s own login rather than making you use the CLI: it starts the
          flow, hands you the URL and types the code back if one is wanted. Codex runs its own
          browser callback, so there <code className="inline">finish_auth</code> waits rather than
          types. If a CLI does something unexpected, whatever it printed comes back verbatim along
          with the command to run yourself.
        </p>
        <p>
          Answers are remembered for a minute, because every probe means running a CLI. A{" "}
          <em>no</em> is only trusted for ten seconds, so a network blip cannot quietly drop a
          provider off your dial.
        </p>
      </section>

      <section className="section" id="limits">
        <h2>limits.</h2>
        <div className="terminal">
          Inference.openai.limits{"\n"}
          <span className="dim">
            # &#123;&apos;5h&apos;: &#123;&apos;used&apos;: 0.0, &apos;reset&apos;: 1787209867&#125;,
          </span>
          {"\n"}
          <span className="dim">
            #{"  "}&apos;7d&apos;: &#123;&apos;used&apos;: 0.16, &apos;reset&apos;: 1787196805&#125;&#125;
          </span>
        </div>
        <p>
          <code className="inline">used</code> is a fraction, so 0.16 is 16%. Every provider is
          asked in a way that costs no tokens.
        </p>
        <div className="grid" style={{ gridTemplateColumns: "auto auto 1fr" }}>
          <div className="gh">provider</div>
          <div className="gh">how</div>
          <div className="gh">worth knowing</div>
          <div className="gc strong">claude</div>
          <div className="gc">get_usage control request</div>
          <div className="gc quiet">some enterprise plans report no windows; used is then None</div>
          <div className="gc strong">openai</div>
          <div className="gc">account/rateLimits/read</div>
          <div className="gc quiet">read by window duration — primary is not always the 5h one</div>
          <div className="gc strong">google</div>
          <div className="gc">agy -p /usage</div>
          <div className="gc quiet">reports remaining per model group; omni reports used, worst first</div>
        </div>
      </section>

      <section className="section" id="providers">
        <h2>providers.</h2>

        <h3>Claude Code</h3>
        <p>
          One long-lived <code className="inline">claude -p</code> handles every turn over stdin.
          Seeding is a file write into{" "}
          <code className="inline">~/.claude/projects/&lt;slug&gt;/&lt;uuid&gt;.jsonl</code>, where
          the slug is the working directory with every non-alphanumeric character turned into a
          dash — resolved through symlinks first, because on macOS <code className="inline">/var</code>{" "}
          is one. Resume is scoped to that directory, so a session pins its cwd. Model and effort
          change over the control channel and omni only believes it once the CLI acknowledges.
        </p>

        <h3>Codex</h3>
        <p>
          The app server, not <code className="inline">exec</code>. It always runs against a{" "}
          <code className="inline">CODEX_HOME</code> under <code className="inline">~/.omni/jails/</code>{" "}
          holding a symlink to your real <code className="inline">auth.json</code> — linked, not
          copied, so a token refresh is not lost — and a near-empty config. Skills live outside
          that folder, so <code className="inline">disable_subagents()</code> switches them off one
          at a time over the protocol. A mid-turn message becomes a{" "}
          <code className="inline">turn/steer</code>, falling back to a new turn if the old one
          finished first.
        </p>

        <h3>Antigravity</h3>
        <p>
          The most restricted, and the one with the sharpest edges. Cold start is around ten
          seconds per launch. An unrecognised conversation id makes agy silently start a new one,
          so omni checks the id it gets back and re-seeds from the top if it is not the one it
          asked for. <code className="inline">--print</code> takes a value and will swallow the
          next flag if it is not passed last. Any failed turn kills the process outright.
        </p>
      </section>

      <section className="section" id="registry">
        <h2>the registry.</h2>
        <p>
          This site serves the dial. omni asks for the one matching the providers it has and keeps
          the answer under <code className="inline">~/.omni/cache</code> for an hour.
        </p>
        <div className="terminal">
          <span className="dim">$ </span>curl{" "}
          <span className="accent">
            &apos;https://omni.teamofsilicons.com/api/intelligence?providers=claude+google&apos;
          </span>
          {"\n\n"}
          &#123;{"\n"}
          {"  "}<span className="accent">&quot;providers&quot;</span>: &quot;claude+google&quot;,{"\n"}
          {"  "}<span className="accent">&quot;ladder&quot;</span>: &#123;{"\n"}
          {"    "}&quot;10&quot;: &#123;&quot;provider&quot;: &quot;claude&quot;, &quot;model&quot;:
          &quot;claude-opus-5&quot;, &quot;effort&quot;: &quot;max&quot;, …&#125;,{"\n"}
          {"    "}…{"\n"}
          {"    "}&quot;0&quot;: &#123;…&#125;{"\n"}
          {"  "}&#125;,{"\n"}
          {"  "}<span className="accent">&quot;ladders&quot;</span>: &#123; every other combination
          &#125;{"\n"}
          &#125;
        </div>
        <p>
          Point omni somewhere else with <code className="inline">OMNI_REGISTRY</code>. If the
          registry cannot be reached, omni prefers a dial it fetched before — even a stale one —
          over the copy packaged with the release, and backs off for five minutes rather than
          retrying on every call.
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

      <Footer />
    </>
  )
}
