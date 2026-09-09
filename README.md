# silicon omni

One persistent interface for **Claude Code**, **Codex** and **Antigravity** — driven by
the subscriptions you already pay for, not API keys. A small Rust daemon keeps the
providers and conversations warm; Python, Rust, and the bundled terminal client all
speak to it.

```python
from omni import Inference, Event

chat = Inference.load_or_create_session("my-session")
chat.model("code")

@chat.on_event
def handle(event):
    if event.type == Event.TEXT:
        print(event.text)

chat.start()
chat.send("what changed in this repo today?")
```

The interesting part is not that it wraps three CLIs. It is that a conversation can
**move between them over its lifetime**, survive the program that opened it, and carry
on where it left off.

---

## Why

Each vendor ships a good agentic CLI and a subscription that makes it cheap to run.
None of them talk to each other. Pick one and you are stuck with its models, its
limits and its outage.

omni owns the conversation instead. Providers become interchangeable: raise the
intelligence dial and the same chat may finish on a different vendor's model, with
everything that came before already in its head.

---

## Install

```bash
pip install silicon-omni
```

The wheel includes `silicon-omni` (also `so`), the terminal client, and `omnid`,
the Rust daemon. The first operation that needs the daemon starts it automatically;
later programs connect to the same Unix socket under `~/.omni`. There is no service
to install and no API server to configure. Set `OMNI_HOME` to move all state, or
`OMNI_DAEMON` to use a particular daemon binary while developing. Release wheels
target manylinux 2.28 on x86_64 and aarch64, plus macOS on Intel and Apple silicon.

Zero runtime dependencies. You bring the CLIs:

| provider | CLI | check |
|---|---|---|
| `claude` | Claude Code | `claude auth status` |
| `openai` | Codex | `codex app-server` |
| `google` | Antigravity | `agy models` |

```python
Inference.get_available_providers()   # ['claude-code-cli', 'antigravity-cli', 'codex-app-server']
```

Installed *and* logged in. Anything else is not offered.

### Terminal and Rust clients

The `silicon-omni` and `so` commands installed by the Python wheel are the same
socket client in a terminal. It starts `omnid` on first use, streams a turn as it
happens, and detaches without cooling the provider. The CLI aliases and daemon can
also be installed through Cargo:

```bash
cargo install omni-daemon silicon-omni-cli
```

```bash
silicon-omni chat my-session
so send --key code my-session "what changed in this repo today?"
so logs --since 42 my-session
so history --since 0 my-session
so sessions
so providers
so dial claude openai
so account claude status
so daemon status
```

Every streaming command accepts `--json`, which emits one `Event` per line; add
`--frames` only when a low-level integration needs transport envelopes and session
snapshots. 0.7 removed the 0.4 aliases `--level`, `--from`, `events` and `attach`. Use `--key`, `--intelligence`, `--model`, `--since`, `history`, and `logs`.
`silicon-omni help` (or `so help`) lists session settings, account login, history,
and daemon lifecycle commands. In a checkout, build them with
`cargo build --release -p omni-daemon -p silicon-omni-cli`.

Rust programs use the synchronous `silicon-omni` crate with the same
`Inference → Chat → Event` path as Python. One connection multiplexes concurrent
replies and any number of Chat streams; independent requests may finish out of order,
while requests for the same session retain their wire order. `start` subscribes before
it asks the daemon for replay, so an early Event cannot be lost:

```bash
cargo add silicon-omni
```

```rust
use silicon_omni::{Ask, Event, Inference};

fn main() -> silicon_omni::Result<()> {
    let inference = Inference::connect()?;
    let mut chat = inference.load_or_create_session("my-session", None);
    chat.model("code")?.start()?;
    chat.send("what changed in this repo today?")?;

    for event in chat.events() {
        let event = event?;
        if event.event_type == Event::TEXT {
            println!("{}", event.text);
        }
        if event.event_type == Event::END {
            break;
        }
    }
    chat.detach()?;
    Ok(())
}
```

`chat.history()` / `history_since(since)` read persisted Events; `start_since(-1)`
attaches for only future Events; and `refresh()` returns the daemon's current snapshot.
Transport-oriented `Client`, `OpenOptions`, `Session`, `Frame`, and `Request` remain
available under `silicon_omni::raw` for integrations that need the wire itself.

---

## Saying what should answer

There is no model picker. There are three ways to say what the work needs, and
exactly one of them per call.

```python
chat.model("code")                                    # a word
chat.model(intelligence=7)                            # a number
chat.model(model="gemini-3.8-flash-low",              # a model
           provider="antigravity-cli", fast=False)
```

**A word** is a shortlist somebody chose, best first, and the first vendor on it
you are signed into answers. Losing a vendor walks you one place down a list
rather than into an argument.

| key | for |
|---|---|
| `fast` | answer soonest |
| `code` | write and change code |
| `design` | make something somebody has to look at |
| `research` | read a lot, and be right |
| `cost` | spend as little as the job allows |
| `general` | no strong opinion |

The lists live in the registry and are edited by hand. That is deliberate: 0.6
scored every model on three normalised axes and took a weighted sum, and it kept
producing answers that were defensible and wrong — three models within one
percent of each other and a hundred Elo apart, and a winner that moved when a
constant did. Ranking is not judgement.

**A number** is the dial, 0 to 10. Every model the three CLIs can run is plotted
by its score on a benchmark against the dollars it measurably cost to earn that
score, and only the **left edge** becomes a dial: a model earns a rung when
nothing else is both better *and* cheaper. `bench` picks the board —
`gdpval-aa`, `terminal-bench`, `agents-last-exam`, `design-arena-full-stack`, or
`artificial-analysis-intelligence-index`. Names carry no version, so
Terminal-Bench 2.1 becoming 3 changes nothing here.

**A model** is you already knowing. The name goes to the CLI verbatim, so a
model released this morning works without omni knowing about it — and it is
answered on the machine, with no network at all.

### Running hot

`fast` asks a CLI for its faster tier.

| provider | how |
|---|---|
| Codex | `-c service_tier="fast"`, a launch flag — so a hot chat rents its own warm app-server and a normal one is untouched |
| Claude Code | a fast mode on its largest models, opted into per session |
| Antigravity | no such thing. Asking is **ignored rather than refused** |

**omni does none of this choosing, and knows the name of no model.** It asks
`omni.teamofsilicons.com/choose.json` with what it was told and which CLIs are
signed in, and keeps the answer in `~/.omni/cache` for an hour. `model` and
`effort` go to the CLI verbatim, so a model released tomorrow needs no release
of this package — only a commit to
[`models.json`](https://github.com/teamofsilicons/omnipotent) in the registry
repo. Point somewhere else with `OMNI_REGISTRY`.

Nothing ships in the wheel as a fallback. A model list baked into a release is a
model list that goes quietly stale, and a wrong recommendation is worse than an
honest refusal — so a machine that has never reached the registry raises
`NoAnswer` rather than guessing. One that has run before keeps working from its
cache, expired or not.

## Events

Everything omni has to say arrives as one `Event`.

```python
@chat.on_event
def handle(event):
    event.type == Event.THINKING
```

| type | carries |
|---|---|
| `Event.START` | `text` — the message that opened this turn |
| `Event.TEXT` | `text` — one completed assistant message |
| `Event.THINKING` | nothing; the model is reasoning |
| `Event.TOOL.CALL` | `tool`, `args`, `id` |
| `Event.TOOL.RESULT` | `tool`, `id`, `result`, `ok` |
| `Event.END` | the turn is over |
| `Event.INJECTED` | `text` — a message that landed mid-turn |
| `Event.ERROR` | `error`, `kind` — `auth` / `limit` / `unavailable` / `crash` from the model or its CLI, plus `stderr` (CLI chatter), `omni` (the engine itself) and `handler` (your callback raised) |
| `Event.SWITCH_PROVIDER` | `provider`, `extra['from']` |
| `Event.NEW_SESSION` | `provider`, `extra['native']` — the provider's own session id |
| `Event.CONFIG` | `text` — a setting changed |

Every serialized event carries `v`, `type`, and `at`. Events committed by the daemon
also carry the omni `session` and a monotonic `seq`; conversational events carry a
`turn` number beginning at zero. Schema version `v=1` is explicit on new records, and
pre-versioned records are read as version 1. The optional `native` map preserves IDs
reported by the provider — Claude message/tool IDs, Codex thread/turn/item IDs, or agy
conversation/step IDs — without pretending those are portable across providers.

Reasoning is deliberately contentless. A `THINKING` event says the model thought; it
never says what. Provider reasoning is signed or encrypted, cannot be replayed anywhere
else, and has no business sitting in your logs.

Handlers run on one thread, in the order things actually happened. A handler that raises
is reported and stepped over — it cannot take the run down.

The names are identical on every public surface. Python calls the discriminator
`event.type`; Rust calls it `event.event_type` because `type` is reserved, but serializes
it as `type`. `event.kind` is only the classification of an `ERROR`, never the Event's
type. `history` means a finite persisted query, while `on_event` and `logs` deliver live
Events. The complete contract, including lifecycle verbs and compatibility aliases, is
in [`docs/TERMINOLOGY.md`](docs/TERMINOLOGY.md).

---

## Sending

```python
chat.send("...")
```

Sends a message. If nothing is running it opens a turn. If a turn is already in flight
it is **injected**: the provider picks it up at the next safe point, once the tool it is
running has finished. `send` returns only after the daemon has durably placed the
message in the session's local pending FIFO. It does not wait for provider delivery,
the model, a tool, or the end of the turn; a crash after success retries the accepted
message when the session reopens.

How you keep the process alive is your business. A loop:

```python
while chat.status in ("busy", "waiting"):
    message = fetch_new_messages()
    if message:
        chat.send(message)
    else:
        time.sleep(0.2)
    if should_stop() and last_event == Event.END:
        chat.stop()
```

…or a subscription:

```python
import asyncio
import nats  # the example's dependency, not omni's

stop_event = asyncio.Event()


async def main():
    nc = await nats.connect("nats://localhost:4222")
    await asyncio.to_thread(chat.start)

    async def on_msg(m):
        # send waits for daemon acceptance, so keep that socket round trip off
        # an async event loop. It does not wait for the model's response.
        await asyncio.to_thread(chat.send, m.data.decode())
        await m.ack()

    await nc.subscribe("agent.msgs", cb=on_msg)
    await stop_event.wait()


asyncio.run(main())
```

`chat.send` is thread-safe. It can block while a daemon is starting, a session is
opening, or the acceptance reply is in flight, so async applications should put it on a
worker thread. More in [`examples/`](examples/).

`status` is `idle` before `start`, then `busy` / `waiting`, then `stopped`. `chat.idle`
means waiting with no turn open and no local callback left to deliver. A failed provider
send can leave a message queued for a later retry even though no work is currently
running; the accompanying `ERROR` explains that state. Sending to a stopped chat raises
rather than dropping the message on the floor.

---

## Nothing changes mid-turn

This is the rule the whole design hangs off.

```python
chat.model("research")            # noted now
chat.active_inference_providers(["claude-code-cli", "codex-app-server"])
chat.system_prompt("...")
chat.enable_subagents()
```

Every call queues a change. The conductor first writes the value to session metadata,
then records a `CONFIG` event, and **applies it at the next turn boundary** — after the
running tool finishes and the turn ends. Once that event is visible, the setting
survives cold reaping and daemon restarts. A model never changes underneath itself.

Calling any of them again overwrites the last value. A non-empty provider list passed to
`load_or_create_session` is an explicit setting too; omitting it restores the session's
persisted list.

### Working directory

The first client to create a session pins its current working directory. Python sends
`os.getcwd()`, and Rust and the terminal default a new Chat to their current directory.
Reopening the session somewhere else does not silently move its tools.

```python
chat.cwd("/another/repository")
```

That explicit change is durable and takes effect at the next turn boundary, using the
same reseed/resume rules as any other configuration change.

### Prompts and isolation

```python
chat.system_prompt("...")            # replace the provider's own prompt
chat.system_prompt_file("p.txt")
chat.append_system_prompt("...")     # or keep theirs and add
```

A chat starts quiet: **no subagents, no MCP servers, no memory files**. A provider that
brings its own help makes the same run mean different things on different machines, so
you opt back in rather than out.

```python
chat.enable_subagents()              # let the provider spawn its own
chat.enable_mcp()                    # let it load MCP servers and connectors
```

Not every provider can honour those. Codex is always jailed, so `enable_mcp()` does
not reach it; agy has no switch for either. The provider that cannot say yes logs a
`CONFIG`/`unsupported` event saying which of your settings it ignored — once, when you
set it and when the conversation arrives there, rather than on every relaunch.

Memory files are not a switch. `CLAUDE.md`, auto memory, org memory and `AGENTS.md`
never load, whichever way the other two are set.

---

## Sessions, clients, and how switching works

`~/.omni/sessions/{id}.jsonl` is the source of truth. It is the event log — the same
objects your handlers see, appended in order. The daemon data-syncs an append before it
publishes the event. A complete final JSON record without its newline is preserved; an
invalid partial tail is truncated before the next append. Complete malformed lines,
invalid events, sequence gaps, and foreign session ids stop that session instead of
being silently skipped. The log outlives any single provider and any single Python
process.

```python
chat = Inference.load_or_create_session("nightly-triage")
```

The daemon owns one live conversation per session id, and any number of clients may
attach to it. Every attached client hears the same ordered event stream, and any of
them may send:

```python
first = Inference.load_or_create_session("nightly-triage").start()
second = Inference.load_or_create_session("nightly-triage").start()

first.send("from the worker")       # both clients hear the answer
second.send("from the dashboard")  # either client can drive the chat
```

`chat.detach()` stops listening but leaves the provider hot. Reopening the id during
the 15-minute idle grace period reconnects to the same running conversation. `stop()`
is deliberately different: it ends the session and shuts its providers down. An open
turn gets exactly one durable `END`; if the provider does not emit it while stopping,
omni writes a synthetic one marked `interrupted` and `stopped`.

By default `start()` replays the whole event log to a newly attached client. Pass the
next sequence number you need to resume exactly, or `since=-1` to hear only new events:

```python
chat.start(since=last_seq + 1)
watcher.start(since=-1)
```

Alongside it, `{id}.meta.json` atomically remembers durable chat settings plus each
provider's **own** session and how far up the omni log it has already seen:

```json
{"pending": [{"id": "8f91…", "text": "accepted, not delivered yet"}],
 "providers": {"claude-code-cli": {"id": "3cb0…", "synced": 19},
               "antigravity-cli": {"id": "1dbc…", "synced": 26}},
 "settings": {"cwd": "/work/nightly", "level": 7,
              "active_providers": ["claude-code-cli", "antigravity-cli"]}}
```

This is the internal version-1 metadata shape, so it retains the key `level` for
on-disk compatibility. Clients normalize it to `intelligence` before exposing it.

`pending` is the transactional FIFO behind `send` acceptance. Its id is copied to
`extra.message_id` on the durable `START` or `INJECTED` event that proves provider
delivery, so crash recovery can reconcile identical messages without guessing by text.
`synced` is a contiguous watermark, not merely the largest sequence observed; late
output from a replaced runner leaves a hole that is supplied when that provider
returns.

State is private to the local account even under a permissive umask: omni-owned
directories are mode `0700`, and the daemon socket, PID/log files, and session JSONL
and metadata are forced to `0600`. Metadata replacement and newly created directory
entries are synced as well as file contents. If history or metadata cannot be
persisted, the session stops rather than publishing the change as durable. Existing
malformed metadata or complete-line log corruption is retained for diagnosis and the
session refuses to launch; it is never overwritten with empty state.

So when a conversation moves:

- **Continuing on the same provider** uses its native resume. omni's log is not read at all.
- **Arriving somewhere new** replays only the part that provider missed.
- **Coming back** resumes its own session and tops it up with what happened while away.

Which is exactly the scenario worth naming: start on Gemini, raise the dial to Opus
mid-way, chat, drop back to Gemini. Gemini picks up its own conversation and is told
what Claude did. Nothing is re-read that does not need to be.

### What crosses, and what it looks like

Providers do not share tools, so a tool the destination does not have is rendered as
text that reads as what happened:

```
[GoogleSearch: "kite festivals"]
[GoogleSearch result: 12 results …]
```

The omni log keeps the structured original, so this form only ever exists inside the
seed handed to somebody else. Going back to Gemini replays Gemini's own session and the
brackets never happened. **Preserved, not lossy.**

Nothing is trimmed on the way in — not the oldest turns, not a forty-thousand
character tool result. A provider arriving late gets the whole conversation.

---

## Auth

```python
Inference.claude_code_cli.auth_status          # 'authenticated' | 'unauthenticated'
print(Inference.claude_code_cli.start_auth())  # the URL to open
Inference.claude_code_cli.finish_auth("code-or-redirect-url")
```

omni drives each CLI's own login rather than making you use the CLI: it starts the
flow, hands you the URL, and types the code back if one is wanted. Codex runs its own
browser callback, so there `finish_auth` waits rather than types and the code is
ignored. If a CLI does something unexpected, whatever it printed is handed back
verbatim — a confusing message you can read beats a silent failure.

One account per provider.

### When a login dies mid-run

An unauthenticated CLI cannot finish the turn it is in. By default omni takes that
provider off the chat, resolves the **same intelligence value** again over whoever is
left, and carries on there — you get an `ERROR`/`auth`, a `CONFIG`/`provider_removed`
and a `SWITCH_PROVIDER`, and the conversation continues on another vendor's model.

```python
chat.disable_autoremoving_unauthenticated_providers()
```

Turn it off and the auth error is reported and the turn simply ends. Either way the
failed turn is not replayed: it is in the log, so the next provider reads it, but
nothing re-runs a tool that may already have run.

## Limits

```python
Inference.codex_app_server.limits
# {'5h': {'used': 0.0,  'reset': '2026-08-21T14:31:07.000Z'},
#  '7d': {'used': 0.16, 'reset': '2026-08-21T10:53:25.000Z'}}
```

`used` is a fraction, `0.16` being 16%. `reset` is an RFC3339 UTC string from every
provider — one of them answers in epoch seconds, and you never have to know which.
Either can be `None`: some plans report no windows, and *nobody said* is not the same
as *nothing spent*. `'unauthenticated'` if you are not signed in.
Every provider is asked in a way that costs no tokens:

| provider | how | note |
|---|---|---|
| `claude` | `get_usage` control request | free; some enterprise plans report no windows, and `used` is then `None` |
| `openai` | `account/rateLimits/read` | read by window duration, never by position — `primary` is not always the 5h one |
| `google` | `agy -p /usage` | reports *remaining* per model group; omni reports *used*, worst group first |

---

## Logging

```python
@chat.logs
def log(event):
    write_somewhere(event.to_dict())
```

Both hooks receive the daemon's complete event stream: every launch, model change,
provider switch, new session, message in, tool call, error, and stop. `logs` additionally
receives a local `ERROR`/`handler` if one of this Python client's callbacks raises. All
of it is the same `Event` type, so it is parsable without a second schema; daemon events
are the same records already on disk in the session file. A delivered user message may
carry `extra.message_id`, and output that arrived from a runner after replacement carries
`extra.late=true`; private runner-routing epochs never leave the conductor.

---

## Per-provider notes

**Claude Code** — flags do the work. Subagents and MCP are off by the command line
unless a chat opts in; slash commands and memory files (`CLAUDE.md`, auto memory, org
memory) are off unconditionally, so a run means the same thing on anyone's machine.
Seeding is a file write into `~/.claude/projects/<slug>/<uuid>.jsonl`; resume then treats
it as real history. Claude's exact replayed-user echo is its delivery acknowledgement;
unrelated control-channel replay text cannot open or reorder a turn. Unknown structured
assistant blocks are retained as textual JSON rather than disappearing. Model and
effort change over the control channel between turns, so re-tuning does not restart
anything or re-read the conversation.

**Codex** — the app server, not `exec`. One warm app server can carry multiple omni
sessions, routing notifications by Codex thread id. It always runs against a
`CODEX_HOME` of its own under `~/.omni/jails/`, holding exactly two things: a symlink
to your real `auth.json` (created even before a first login, and linked rather than
copied so credentials and token refreshes persist) and a near-empty `config.toml`.
That folder *is* the isolation — codex has nothing left to auto-load, so MCP servers,
hooks and `AGENTS.md` never appear whether or not you asked. Skills live outside
`CODEX_HOME` entirely, so they are switched off one at a time over the protocol; a
failed listing or write is retried rather than remembered as success.
`project_doc_max_bytes=0` goes on every launch, so opting back into subagents cannot
smuggle somebody's `AGENTS.md` in with them. History is seeded with
`thread/inject_items`; a forgotten thread is replaced and fully reseeded. Model and
effort are per-turn parameters, so re-tuning is free.

**Antigravity** — the most restricted. There is no flag for MCP, no flag for subagents,
and no way to seed history, so omni lets it load what it wants and folds prior
conversation into the front of the next message — one turn, not two. Neither isolation
switch can be honoured here; omni logs a `CONFIG` event saying so rather than
pretending. Replacement and appended system prompts are both folded into that text, in
order. An injected message runs as its own native turn instead of joining the one in
flight, and there is no way to interrupt agy at all. Its cold start is ~10s per launch,
and startup is not accepted until agy reports a resumable conversation id. An
unrecognised conversation id makes agy silently start a new one, so omni checks the id
it gets back and re-seeds from the top if it was not the one it asked for.

---

## What omni will not do

- **Reasoning is never carried.** It is signed or encrypted per vendor and cannot be
  replayed anywhere else. omni records that thinking happened and moves on.
- **Tools are observed, not defined.** omni does not install tools into a provider or
  rename theirs. Whatever the CLI does, omni reports.
- **One account per provider.** No multi-account support.
- **A switch may cost a context read.** Providers that can be caught up in place stay
  parked and warm. Claude must restart when it missed history; model changes within a
  provider are re-tuned where its protocol permits.

---

## Contributing

The public Python package is intentionally thin. The daemon and provider protocols live
in Rust:

```
omni/
  chat.py          Python session handle and callback delivery
  client/          Unix-socket transport and automatic daemon startup
  events.py        Python view of the shared event vocabulary
  inference.py     the front door: sessions, accounts, providers, dial
  providers/test.py  control surface for the shipped test provider
crates/
  omni-core/       conductor, sessions, translation, and provider adapters
  omni-daemon/     session registry and NDJSON-over-Unix-socket server
  omni-client/     multiplexed synchronous Rust daemon client
  omni-cli/        terminal client and streaming chat UI
```

The dial, the landing page and the reference live in
[teamofsilicons/omnipotent](https://github.com/teamofsilicons/omnipotent). Which models
exist is that repo's problem; running them is this one's.

Adding a provider means an `Account` and a `Runner` — see
[`crates/omni-core/src/providers/base.rs`](crates/omni-core/src/providers/base.rs) and
the three adapters beside it.
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) explains why the pieces are shaped this way.

```bash
cargo test --workspace        # core, adapters, protocol, conductor
pytest                        # Python through a real daemon; no vendor CLI needed
pytest -m live                # real CLIs; needs auth and spends a little quota
python3 scripts/cleanup.py    # afterwards: takes live-test sessions out of ~/.omni
```

Live tests deliberately run against your real `~/.omni`, because a test that uses
different paths from a real run is not testing a real run. Everything else gets a home
of its own and never touches the network.

For your *own* tests there is a provider that needs no CLI, no login and no quota, and
answers the same way every time:

```python
from omni import Inference
from omni.providers import test

test.install()                                    # registers it, pins a whole 0-10 dial
chat = Inference.load_or_create_session("t", ["test"])
chat.start()
chat.send("hello")            # -> TEXT  'echo: hello'
chat.send("[tool:ls]")        # -> TOOL.CALL + TOOL.RESULT
chat.send("[recall]")         # -> everything it has been told, seeded history included
```

It is not registered until you call `install()`, so it can never appear in
`get_available_providers()` by accident.

For the unhappy paths, `test.running()` hands you the live runner. It records what it
was seeded with and what it was sent, and it can be driven by hand:

```python
test.install("alpha", "beta")            # two of them, so you can test a switch
test.running("alpha").autoreply = False  # hold the turn open
chat.send("hello")
test.running("alpha").fail("auth")       # now lose the login
```

Each knob mimics something a real CLI does. `defer` is agy, which only sees history when
the next message goes out. `tunable = False` is agy again, which cannot change model
without a restart. A native id starting with `gone-` is any provider that has forgotten a
session omni thinks it still has.

MIT.
