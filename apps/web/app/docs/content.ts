/**
 * Everything omni exposes, and what each thing does underneath.
 *
 * `inside` is the optional half: the flag, the JSON-RPC call, the file write
 * that the friendly line actually turns into. It is kept next to the surface
 * rather than in a separate document so the two cannot drift.
 */

export interface Entry {
  call: string
  takes?: string
  gives?: string
  does: string
  inside?: string
}

export interface Section {
  id: string
  /** phosphor icon name, so the contents can be scanned by shape */
  icon: string
  title: string
  blurb: string
  code?: string
  lang?: "python" | "bash" | "http" | "rust"
  entries?: Entry[]
  note?: string
}

export const SECTIONS: Section[] = [
  {
    id: "install",
    icon: "DownloadSimple",
    title: "Install",
    blurb:
      "The Python wheel has no Python dependencies and contains the daemon plus both terminal names. Rust uses the same product name for its library. You bring the provider CLIs; omni offers only the ones installed and signed in.",
    code: `pip install silicon-omni

# Rust library, daemon, and terminal client
cargo add silicon-omni@0.7.0
cargo install omni-daemon@0.7.0 silicon-omni-cli@0.7.0`,
    lang: "bash",
    entries: [
      {
        call: "from omni import Inference, Event",
        takes: "Python",
        does:
          "The original API. The wheel also installs `silicon-omni`, its short alias `so`, and `omnid`; the first operation starts the daemon if it is not already listening.",
      },
      {
        call: "silicon-omni · so",
        takes: "terminal",
        does: "Two names for the same native socket client. The experimental `omni` command name was retired in 0.5.",
      },
      {
        call: "omnid",
        takes: "daemon",
        does:
          "The persistent Rust process that owns sessions and providers. The Python wheel bundles it; a standalone Rust installation gets it from `omni-daemon`.",
        inside:
          "Listens on a private Unix socket under `~/.omni`. Python, Rust and terminal clients all speak the same newline-delimited protocol to it, so a conversation is not owned by whichever client opened it.",
      },
      {
        call: "claude-code-cli",
        takes: "Claude Code",
        does: "Serves Anthropic models. Checked with `claude auth status`.",
        inside:
          "Runs as one long-lived `claude -p --output-format stream-json --input-format stream-json --verbose --dangerously-skip-permissions --disable-slash-commands`, with `--model` and `--effort` taken from the registry's answer. Memory files are switched off through the environment on every run, so a session means the same thing on any machine.",
      },
      {
        call: "codex-app-server",
        takes: "Codex",
        does: "Serves OpenAI models. Checked with an `account/read` over the app server.",
        inside:
          "`codex app-server --stdio`, spoken to as JSON-RPC over newline-delimited JSON. CODEX_HOME is redirected to `~/.omni/jails/codex`, which holds a symlink to your real auth.json and a near-empty config. No MCP server, hook or AGENTS.md is ever loaded.",
      },
      {
        call: "antigravity-cli",
        takes: "Antigravity",
        does: "Serves Gemini models, plus the Anthropic and OpenAI models agy resells. Checked with `agy models`.",
        inside:
          "`agy --output-format stream-json --input-format stream-json --disable-slash-commands --print-timeout 24h --dangerously-skip-permissions --print \"\"`. The empty `--print` goes last on purpose: it takes a value, and anywhere else it swallows the flag that follows it.",
      },
    ],
  },
  {
    id: "clients",
    icon: "TerminalWindow",
    title: "Rust & terminal",
    blurb:
      "All three public surfaces attach to the same daemon and use the same vocabulary. Rust gets a synchronous, multiplexed client; the terminal gets the same client behind two command names.",
    code: `use silicon_omni::{Event, Inference};

fn main() -> silicon_omni::Result<()> {
    let inference = Inference::connect()?;
    let mut chat = inference.load_or_create_session("nightly-triage", None);
    chat.model("code")?.start()?;
    chat.send("what changed today?")?;

    for event in chat.events() {
        let event = event?;
        if event.event_type == Event::TEXT { println!("{}", event.text); }
        if event.event_type == Event::END { break; }
    }
    chat.detach()?;
    Ok(())
}`,
    lang: "rust",
    entries: [
      {
        call: "Inference::connect()",
        gives: "Inference",
        does:
          "Connects to the daemon for this `OMNI_HOME`, starting `omnid` if needed. The Rust client currently uses Unix-domain sockets, so it supports macOS and Linux.",
        inside:
          "One reader thread multiplexes replies and session streams over one connection. Independent requests may finish out of order; requests for one session keep their wire order.",
      },
      {
        call: "inference.load_or_create_session(id, providers)",
        gives: "Chat",
        does:
          "Names a durable session and returns a lazy Chat handle. `chat.model('code')?.start()?` applies queued settings and attaches; `start_since(-1)` asks for only new Events.",
        inside:
          "The subscription is installed before the open request is sent, so replay or an early live frame cannot slip between the reply and the listener.",
      },
      {
        call: "chat.send(text) · .events() · .detach() · .stop()",
        does:
          "Sends, reads ordered Events, leaves the shared session warm, or ends it. `detach` and `stop` deliberately mean different things.",
      },
      {
        call: "chat.history() · .history_since(since) · .refresh()",
        does: "Reads persisted Events or refreshes the daemon-owned Chat snapshot.",
      },
      {
        call: "silicon_omni::raw",
        gives: "advanced transport API",
        does:
          "Keeps `Client`, `OpenOptions`, `Session`, `Frame`, and `Request` available for integrations that need the wire itself; application code begins with `Inference`.",
      },
      {
        call: "silicon-omni chat · so send · so logs",
        does:
          "Starts an interactive Chat, sends and streams one settled turn, or follows the complete live Event log. Use `--key code`, or `--intelligence 7` with an optional `--bench`, or `--model` with `--provider`, `--effort` and `--fast`. Saying two of the three is refused rather than resolved by precedence. `--since` gives the first sequence to replay.",
      },
      {
        call: "so history · sessions · providers · dial · account · daemon",
        does:
          "Reads persisted Events once, or inspects live sessions, available logins, routing, quotas, and the daemon itself. `--json` emits one Event per line; `--frames` opts into transport envelopes and snapshots.",
      },
    ],
    note:
      "0.6 spent one release asking for three weights and a board. It is gone: weighting three axes produced answers that were defensible and wrong. `--intelligence <0-10>` means a dial position again, as it did in 0.5, and sits beside `--key` and `--model`. 0.7 removed the 0.4 aliases `--level`, `--from`, `events` and `attach` rather than carrying them a fourth release.",
  },
  {
    id: "inference",
    icon: "Door",
    title: "Inference",
    blurb: "The front door, and the only object you import besides the events.",
    code: `from omni import Inference, Event

PROVIDERS = Inference.get_available_providers()
chat = Inference.load_or_create_session("nightly-triage", PROVIDERS)`,
    entries: [
      {
        call: "Inference.get_available_providers(limit_to=None)",
        gives: "list[str]",
        does:
          "Lists the providers whose CLI is installed and logged in, in a stable order. Pass `limit_to` to narrow what omni may use.",
        inside:
          "The daemon checks for the binary, then asks the provider's own account protocol. A yes is remembered for 10 minutes, because every probe means running a CLI. A no is trusted for 10 seconds, so a repaired login returns quickly.",
      },
      {
        call: "Inference.load_or_create_session(id, providers=None)",
        gives: "Chat",
        does:
          "Names a session, creating it on `start` if this is the first time. Several programs may attach to the same id; every one hears the ordered events, and any one may send.",
        inside:
          "Constructs a lightweight Python `Chat`. `start()` opens it over the daemon socket; the daemon registry owns the single live engine for that id and adds this connection as another listener. `SessionBusy` remains importable for compatibility but is no longer raised.",
      },
      {
        call: "Inference.dial(providers=None)",
        gives: "dict",
        does: "Returns what the registry answers — by key, by number, or for a model you name — for this provider set. The same call omni routes by.",
      },
      {
        call: "Inference.sessions() · Inference.daemon()",
        gives: "list · dict",
        does: "Lists the sessions the daemon is holding open, or reports the daemon's version, pid and home.",
      },
      {
        call: "Inference.claude_code_cli · .codex_app_server · .antigravity_cli",
        gives: "Account",
        does: "Gives the three account handles: `installed`, `auth_status`, `available`, `start_auth()`, `finish_auth()`, `limits`.",
        inside:
          "Each property is a typed socket request. The daemon owns account probes and their cache, so every client sees the same answer.",
      },
    ],
  },
  {
    id: "chat",
    icon: "ChatCircleDots",
    title: "Chat",
    blurb:
      "Once attached, settings are durably written when you call them and applied at the next turn boundary. Nothing changes mid-turn. The one exception is `send`, which is injected where you sent it.",
    entries: [
      {
        call: "chat.start()",
        gives: "Chat",
        does:
          "Attaches, applies settings queued before the open, and starts hearing events. `since=0` replays the whole log; `since=-1` hears only new events.",
        inside:
          "A warm open adds a listener and replays the requested range. A cold open waits for durable settings and one eager provider launch before the daemon replies, so the next `send` is never trapped behind startup.",
      },
      {
        call: "chat.send(text)",
        does:
          "Opens a turn, or lands inside the one already running. Waits for durable daemon acceptance, not for model output or `END`. Safe from any thread, including an event handler. Raises if the chat is stopped.",
        inside:
          "Crosses the Unix socket and enters a transactionally rewritten metadata FIFO before the reply. A correlated `START` or `INJECTED` event proves delivery and removes it. claude gets a `{\"type\":\"user\"}` line; codex gets `turn/steer` or `turn/start`; agy gets `{\"event\":\"user\"}`.",
      },
      {
        call: "chat.stop()",
        does: "Ends the shared session and shuts its providers down. Safe to call from inside a handler.",
        inside:
          "The daemon acknowledges the stop only after it has committed the lifecycle change. An open turn gets one durable `END`; if the provider emits none, omni writes an interrupted, stopped one.",
      },
      {
        call: "chat.detach()",
        does:
          "Stops this client listening without ending the conversation. Reopen the id during the default 15-minute grace period and its active provider is still warm.",
        inside:
          "The daemon removes only this connection's listener. Process exit has the same ownership effect; an idle session with no listeners is reaped after `OMNI_IDLE_SECS`, 900 seconds by default.",
      },
      {
        call: "chat.history(since=0) · chat.refresh()",
        gives: "list[Event] · dict",
        does: "Reads persisted events from a sequence number, or refreshes the local status snapshot from the daemon.",
      },
      {
        call: "chat.status",
        gives: "str",
        does: "Reports `idle` before start, then `busy` / `waiting`, then `stopped`.",
      },
      {
        call: "chat.idle",
        gives: "bool",
        does: "True when the chat is waiting and nothing is queued. This is the one a polling loop should check.",
      },
      {
        call: "chat.model(key) · (intelligence=n, bench=…) · (model=…, provider=…, effort=…, fast=…)",
        takes: "a word, a number, or a model",
        does: "Says what should answer. A key is a shortlist chosen by hand; a number is the dial; a model is passed to the CLI verbatim. May change model, effort and vendor at once.",
        inside:
          "Records the wish and returns. At the next turn end the wanted `(provider, model, effort, config)` is compared with what the runner was built with. If only model or effort differ on the same provider, the runner is re-tuned in place; anything else is a restart.",
      },
      {
        call: "chat.active_inference_providers([...])",
        does: "Narrows which providers omni may route to.",
        inside: "Changes which dial is asked for, since there is one dial per set of providers.",
      },
      {
        call: "chat.system_prompt(text)",
        does: "Replaces the provider's own session prompt.",
        inside:
          "claude `--system-prompt`; codex `baseInstructions` on thread start. agy has no equivalent, so it is folded into the front of the first message and a `config` event records that it was approximated.",
      },
      { call: "chat.system_prompt_file(path)", does: "Reads the same from a file." },
      {
        call: "chat.append_system_prompt(text)",
        does: "Keeps the provider's prompt and adds to it.",
        inside: "claude `--append-system-prompt`; codex `developerInstructions`.",
      },
      { call: "chat.append_system_prompt_file(path)", does: "Reads the same from a file." },
      {
        call: "chat.enable_subagents()",
        does:
          "Lets the provider spawn its own. Off by default: a chat starts quiet and you opt in, never out.",
        inside:
          "Quiet means claude gets `--disallowedTools \"Agent(*)\"` plus `CLAUDE_CODE_DISABLE_WORKFLOWS=1`, and codex gets `--disable apps --disable plugins -c agents.enabled=false`; because skills live outside CODEX_HOME they are additionally switched off one at a time over `skills/config/write`. `-c project_doc_max_bytes=0` is sent whichever way this is set, so opting in cannot bring an AGENTS.md along with it. agy gets `--disable-slash-commands`, which switches its skills off but not its subagents, and it has no way at all to stop a GEMINI.md or AGENTS.md in the working directory being read — so a quiet chat is quieter on the other two.",
      },
      {
        call: "chat.enable_mcp()",
        does: "Lets the provider load its MCP servers and connectors. Off by default.",
        inside:
          "Quiet means claude gets `--strict-mcp-config --setting-sources \"\"`. codex is isolated by its jail either way. agy is not: omni passes it no environment today, so it loads whatever MCP servers and plugins the machine has. `agy` does honour `HOME`, so the same jail trick works on it — it is a gap in omni, not in the CLI.",
      },
      {
        call: "chat.disable_subagents()",
        does: "States the default out loud.",
      },
      { call: "chat.disable_mcp()", does: "Likewise." },
      {
        call: "chat.disable_autoremoving_unauthenticated_providers()",
        does:
          "Stops dropping a provider that loses its login mid-run. The dropping is on by default; see Auth.",
      },
      {
        call: "chat.enable_autoremoving_unauthenticated_providers()",
        does: "Restores the default auth-failover behaviour.",
      },
      {
        call: "chat.cwd(path)",
        does: "Sets where the provider runs its tools. Pinned to the session.",
        inside:
          "Resolved through symlinks first. Claude names its session file after the working directory, and on macOS `/var` and `/tmp` are links, so an unresolved path files the session somewhere omni would never look again.",
      },
      {
        call: "@chat.on_event",
        does: "Decorator. Delivers every event as it happens, on one thread, in order.",
        inside:
          "A handler that raises is reported as an `ERROR` of kind `handler` and stepped over. It cannot take the run down.",
      },
      {
        call: "@chat.logs",
        does:
          "Decorator. Delivers the same complete daemon Event stream as `on_event`, plus a local `ERROR`/`handler` if one of this Python client's callbacks raises.",
        inside:
          "Callbacks run in order on this Python session's dispatcher. A failed event handler becomes a local `handler` error for log subscribers; a failed log handler is printed and cannot affect the daemon.",
      },
    ],
  },
  {
    id: "events",
    icon: "Broadcast",
    title: "Events",
    blurb:
      "One versioned type for everything. The same ordered daemon Events go to every attached Python, Rust, and terminal client and onto disk, so the session file is the event log and there is never a second schema.",
    entries: [
      { call: "Event.START", takes: "text", does: "The message that opened this turn." },
      { call: "Event.TEXT", takes: "text", does: "One finished assistant message." },
      {
        call: "Event.THINKING",
        does: "The model is reasoning. Never what it thought.",
        inside:
          "Reasoning is signed or encrypted per vendor and cannot be replayed anywhere else, so omni records that it happened and drops the content, including out of your logs. There is no flag to keep it.",
      },
      { call: "Event.TOOL.CALL", takes: "tool, args, id", does: "A tool was invoked." },
      { call: "Event.TOOL.RESULT", takes: "tool, id, result, ok", does: "It came back." },
      { call: "Event.END", does: "The turn is over." },
      { call: "Event.INJECTED", takes: "text", does: "A message that landed inside a running turn." },
      {
        call: "Event.ERROR",
        takes: "error, kind",
        does:
          "`auth` · `limit` · `unavailable` · `crash` from the model or its CLI, plus `stderr` for CLI chatter, `omni` when the engine failed, and `handler` when your own callback raised.",
        inside:
          "Every provider words its failures differently. omni sorts them into the only four answers that change what you do: log in, wait, retry, or read a stack trace.",
      },
      { call: "Event.SWITCH_PROVIDER", takes: "provider, extra.from", does: "The conversation moved." },
      { call: "Event.NEW_SESSION", takes: "extra.native", does: "A provider opened one of its own." },
      {
        call: "Event.CONFIG",
        takes: "text",
        does:
          "A setting changed, or omni did something worth writing down: `launch` · `retune` · `reseed` · `provider_removed` · `unsupported` · `approximated` · `stop`.",
      },
      {
        call: "event.seq",
        gives: "int",
        does:
          "Its position in the session log. It only goes up, and it never repeats, across every provider the conversation has passed through.",
        inside:
          "This is how omni knows what a provider still has to be told. The meta file records the last seq each one saw, so coming back replays exactly what was recorded since, and nothing twice.",
      },
    ],
    note: "Fields: v, type, session, provider, model, text, tool, id, args, result, ok, kind, error, at, seq, turn, native, extra. Python exposes `event.type`; Rust exposes `event.event_type` because `type` is reserved but serializes it as `type`. `kind` only classifies an error. `v` is the on-disk schema version; `turn` groups cross-provider activity; `native` retains provider ids without pretending they are portable history. Anything at its default is left out rather than written as null.",
  },
  {
    id: "sessions",
    icon: "Files",
    title: "Sessions",
    blurb:
      "The session file is the source of truth and it outlives every provider and client. The daemon owns live state; everything else on disk is durable bookkeeping.",
    entries: [
      {
        call: "~/.omni/sessions/{id}.jsonl",
        does: "The conversation, as an append-only, data-synced event log. Never rewritten.",
        inside:
          "A live session keeps one validated appender open. Every record still crosses `sync_data` before publication; a torn tail is repaired, while complete corruption is preserved and refuses the session.",
      },
      {
        call: "~/.omni/sessions/{id}.meta.json",
        does:
          "Durable settings, accepted messages awaiting delivery, and each provider's own session id and contiguous log watermark.",
        inside:
          "`pending` is the FIFO behind send acceptance. `synced` advances only across contiguous history that provider has seen. Identical metadata is not rewritten.",
      },
      {
        call: "~/.omni/omnid.sock · omnid.pid · omnid.log",
        does: "The private Unix socket, daemon identity, and daemon diagnostics shared by every client.",
      },
      { call: "~/.omni/jails/codex", does: "Codex's stripped, shared CODEX_HOME." },
      { call: "~/.omni/cache/choose.json", does: "The answer, keyed by what was asked and which providers are signed in, kept an hour." },
    ],
    note:
      "Consecutive turns on one provider keep one live runner and native conversation. A switch may pay activation cost. Arriving somewhere new gets only what it missed; coming back resumes its own session and catches up. Codex can catch up in place, Antigravity carries missed history in its next message, and stale Claude resumes through a replacement process.",
  },
  {
    id: "performance",
    icon: "Timer",
    title: "Hot sessions",
    blurb:
      "The 0.5 performance contract is steady-state warmth: consecutive turns on Claude, Codex or Antigravity reuse one runner and one native conversation. Switching providers may cost an activation.",
    entries: [
      {
        call: "live workload",
        does:
          "Teach Claude five facts over five messages, then Codex five, then Antigravity five, then ask Claude for the exact compact fifteen-fact array. Five fresh, alternating trials per version.",
      },
      {
        call: "correctness · 0.3 and 0.5",
        gives: "5/5 · 80/80",
        does:
          "Both versions passed every session and turn with exact `ACK` teaching replies and exact final recall. PID and native identity stayed fixed within every five-turn provider group.",
      },
      {
        call: "whole workflow · median",
        gives: "49.543s → 44.826s",
        does: "0.5 was 9.5% faster than 0.3 across the complete sixteen-turn workflow; the mean improved 6.6%.",
      },
      {
        call: "0.5 · first turn → hot turns 2–5",
        does:
          "Claude `2.663s → 1.368s` (1.95×); Codex `3.288s → 1.184s` (2.78×); Antigravity `13.727s → 1.940s` (7.08×).",
        inside:
          "Stable PID and native identity are the decisive heat signal. The remaining 1.2–1.9 seconds for a tiny `ACK` is provider inference, not a process boot.",
      },
      {
        call: "hot 0.3 → hot 0.5",
        does:
          "Claude changed `1.251s → 1.368s`, Codex `1.208s → 1.184s`, and Antigravity `1.808s → 1.940s`. Mixed directions mean 0.5 is not claiming a blanket cloud-inference speedup.",
      },
      {
        call: "Python send() acceptance",
        gives: "0.033ms → 10.801ms",
        does:
          "The contracts differ: 0.3 enqueued in-process; 0.5 crosses IPC and returns after durable journaling. That roughly 11ms local commit is small beside a model turn.",
      },
    ],
    note:
      "The timed client was Python. Rust and the CLI use the same daemon engine but were not scored as separate inference benchmarks. These are five descriptive live trials with frozen CLI/model versions; method and results live at `benchmarks/results/2026-08-25-v0.3.0-v0.5.0.md` in the silicon-omni repository.",
  },
  {
    id: "auth",
    icon: "Key",
    title: "Auth",
    blurb: "omni drives each CLI's own login, so you never have to open the CLI yourself.",
    code: `Inference.claude_code_cli.auth_status
print(Inference.claude_code_cli.start_auth())      # the url to open
Inference.claude_code_cli.finish_auth("code-or-redirect-url")`,
    entries: [
      {
        call: "start_auth()",
        gives: "str",
        does: "Begins a login and hands back the URL to open.",
        inside:
          "Spawns the CLI's own login, watches its output for a URL and returns it. If nothing usable appears, whatever it printed comes back verbatim, along with the command to run by hand.",
      },
      {
        call: "finish_auth(code)",
        gives: "str",
        does: "Takes the code or redirect URL back, then re-checks.",
        inside:
          "Codex runs its own browser callback, so there this waits for an `account/login/completed` notification instead of typing anything, and the code you pass is ignored.",
      },
      {
        call: "a login that dies mid-run",
        does:
          "The provider is dropped from that Chat, the error is reported, and the same intelligence value is resolved again over whoever is left. The conversation carries on somewhere else, at an intelligence value that now means something slightly different.",
        inside:
          "An `ERROR`/`auth` closes the turn, emits `CONFIG`/`provider_removed`, then relaunches, which reports as a `SWITCH_PROVIDER`. The failed turn is not replayed: it is in the log for the next provider to read, but re-driving it could re-run a tool that has already run. Switch the whole behaviour off with `chat.disable_autoremoving_unauthenticated_providers()` and the error ends the turn and nothing else.",
      },
    ],
  },
  {
    id: "limits",
    icon: "Gauge",
    title: "Limits",
    blurb:
      "Asking costs no tokens on any of the three. `used` comes back a fraction and `reset` an RFC3339 UTC string, whatever each one answers natively: one of them counts seconds since the epoch, and you never have to know which.",
    code: `Inference.codex_app_server.limits
# {'5h': {'used': 0.0,  'reset': '2026-08-21T14:31:07.000Z'},
#  '7d': {'used': 0.16, 'reset': '2026-08-21T10:53:25.000Z'}}`,
    entries: [
      {
        call: "claude-code-cli",
        does: "Some enterprise plans expose no windows at all. `used` is then None, and not zero — the difference matters.",
        inside: "A `get_usage` control request over a throwaway stream-json process. Costs nothing and needs no credentials of omni's own.",
      },
      {
        call: "codex-app-server",
        does: "Read by window duration, never by position.",
        inside:
          "`account/rateLimits/read`. `primary` is not always the 5-hour window; on some plans it is the weekly one. So omni branches on `windowDurationMins`, where 300 is the 5h and 10080 the 7d.",
      },
      {
        call: "antigravity-cli",
        does: "Reports remaining per model group. omni reports used, worst group first.",
        inside: "`agy -p /usage`, which the CLI answers itself, so no model is called and no quota is spent.",
      },
    ],
  },
  {
    id: "testing",
    icon: "TestTube",
    title: "Testing",
    blurb:
      "A provider that needs no CLI, no login and no quota, and answers the same way every time. It is not registered until you ask for it, so it cannot turn up in a real run by accident.",
    code: `from omni import Inference
from omni.providers import test

test.install()
chat = Inference.load_or_create_session("t", ["test"])
chat.start()
chat.send("hello")        # -> TEXT  'echo: hello'`,
    entries: [
      {
        call: "test.install()",
        gives: "list[str]",
        does: "Registers the provider and pins a fixed answer for it, however it is asked for.",
        inside:
          "The Python call crosses the real socket and registers the Rust test provider inside `omnid`, then pins a dial for it. Routing, persistence and events run through the shipped engine without reaching the network.",
      },
      { call: "[tool:NAME]", does: "Runs `NAME`, and emits a `TOOL.CALL` and a matching `TOOL.RESULT`." },
      {
        call: "[recall]",
        does: "Replies with everything it was told before this message.",
        inside:
          "Including history it was only ever seeded with, never sent, which is what makes it useful for testing a provider switch without switching provider.",
      },
      { call: "anything else", does: "Replies `echo: <what you sent>`." },
    ],
    note:
      "The library's own live suite runs against the real ~/.omni rather than a sandbox, because a test that uses different paths from a real run is not testing a real run. `scripts/cleanup.py` in the repo takes its sessions, jails and working directories back out afterwards.",
  },
  {
    id: "registry",
    icon: "Database",
    title: "The registry",
    blurb:
      "omni contains the name of no model and does none of this choosing. It sends a key, a number, or a model it was given, plus signed-in CLIs and discovered model availability. The registry follows the configured priority and skips access-gated models unless their exact IDs are confirmed available.",
    code: `GET https://backend.omni.teamofsilicons.com/choose.json?key=code&providers=claude-code-cli+antigravity-cli`,
    lang: "http",
    entries: [
      {
        call: "OMNI_REGISTRY",
        does: "Points omni at your own registry. The contract is one GET with a `providers` query.",
        inside:
          "The answer is cached under `~/.omni/cache` for an hour, keyed by the set of providers. A dial fetched once is reused even after it has expired, so a machine that has run before keeps working offline.",
      },
      {
        call: "NoAnswer",
        does:
          "Raised when the registry has never been reached and nothing is cached. No model list ships in the wheel as a fallback, so this is a hard stop and not a degraded mode.",
        inside:
          "A model list baked into a release is a list going stale, and quietly recommending last quarter's best buy is worse than refusing to answer.",
      },
    ],
  },
]
