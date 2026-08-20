# silicon omni

One Python interface for **Claude Code**, **Codex** and **Antigravity** — driven by the
subscriptions you already pay for, not API keys.

```python
from omni import Inference, Event

chat = Inference.load_or_create_session("my-session")
chat.intelligence(7)

@chat.on_event
def handle(event):
    if event.type == Event.TEXT:
        print(event.text)

chat.start()
chat.send("what changed in this repo today?")
```

The interesting part is not that it wraps three CLIs. It is that a conversation can
**move between them mid-flight** and carry on where it left off.

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

Zero runtime dependencies. You bring the CLIs:

| provider | CLI | check |
|---|---|---|
| `claude` | Claude Code | `claude auth status` |
| `openai` | Codex | `codex app-server` |
| `google` | Antigravity | `agy models` |

```python
Inference.get_available_providers()   # ['claude', 'google', 'openai']
```

Installed *and* logged in. Anything else is not offered.

---

## The dial

There is no model picker. There is one number.

```python
chat.intelligence(0)    # cheapest thing worth using
chat.intelligence(10)   # best thing you have
```

Behind it is a graph. Every model our three CLIs can run is plotted by its
[GDPval-AA v2](https://artificialanalysis.ai/evaluations/gdpval-aa) Elo — Artificial
Analysis' blind pairwise scoring of real economically valuable work, anchored so that a
human expert is 1000 — against the dollars they measured it cost to earn that score.

Only the **left edge** of that graph becomes a dial: a model earns a level if nothing
else is both better *and* cheaper. Level 10 is the top of the edge, and the dial walks
down-left from there, so every step down is a real saving and never a sideways move.

```
lvl    Elo   $/task   model
 10  1844.7  6.7660   claude-opus-5 max
  9  1813.8  4.9630   claude-opus-5 xhigh
  8  1732.8  3.0271   claude-opus-5 high
  7  1678.9  2.1141   gpt-5.6-sol xhigh
  6  1621.2  1.3708   gpt-5.6-sol high
  5  1619.6  1.3641   claude-opus-5 medium
  4  1578.3  0.1022   gpt-5.6-luna max
  3  1525.7  0.0667   gpt-5.6-luna xhigh
  2  1465.8  0.0412   gpt-5.6-luna high
  1  1274.5  0.0133   gpt-5.6-luna medium
  0  1155.6  0.0072   gpt-5.6-luna low
```

Elo is anchored so a human expert scores 1000, over 220 real work tasks in an agentic
harness; the dollars are what Artificial Analysis measured those runs cost. Level 4 is
worth staring at: GPT-5.6 Luna at max effort scores within 15% of the top of the board
for **66× less money**, which is why everything between it and Opus 5 falls off the edge.

There is **one dial per set of providers**, because losing a provider puts models back on
the dial that another vendor's were shadowing. With fewer providers the edge is shorter
and levels start sharing a rung — that is the dial telling you there is nothing in
between worth picking.

omni does none of that arithmetic. It asks `omni.teamofsilicons.com` for the dial
matching the providers it has, and keeps the answer in `~/.omni/cache` for an hour.
Point it at your own with `OMNI_REGISTRY`; the registry itself lives in
[`docs/`](docs/) and deploys to Vercel.

That host does not exist yet, so today every dial comes from the packaged
[`omni/intelligence/ladder.json`](omni/intelligence/ladder.json) — a plain map from level
to model, one per provider set. It is a bootstrap, not a fallback: check its `captured`
date, because a dial that has gone stale will quietly keep recommending last month's best
buy. Once a real dial has been fetched, omni prefers it over the packaged one even after
it expires. `model` and `effort` go to the CLI verbatim, so a new model release is new
data and no new code.

Choosing which models belong on the dial is
[`tools/build_ladder.py`](tools/build_ladder.py)'s job, not omni's. Run it to rebuild the
packaged file when the leaderboard moves. Its `caveats` list says exactly which models
were left off and why: anything GDPval has not scored *or* costed is left off rather than
guessed at.

---

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
| `Event.ERROR` | `error`, `kind` — `auth` / `limit` / `unavailable` / `crash`, plus `omni` (the engine itself) and `handler` (your callback raised) |
| `Event.SWITCH_PROVIDER` | `provider`, `extra['from']` |
| `Event.NEW_SESSION` | `extra['native']` — the provider's own session id |
| `Event.CONFIG` | `text` — a setting changed |

Reasoning is deliberately contentless. A `THINKING` event says the model thought; it
never says what. Provider reasoning is signed or encrypted, cannot be replayed anywhere
else, and has no business sitting in your logs.

Handlers run on one thread, in the order things actually happened. A handler that raises
is reported and stepped over — it cannot take the run down.

---

## Sending

```python
chat.send("...")
```

Sends a message. If nothing is running it opens a turn. If a turn is already in flight
it is **injected**: the provider picks it up at the next safe point, once the tool it is
running has finished. Either way `send` returns immediately.

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
import nats  # the example's dependency, not omni's

stop_event = asyncio.Event()


async def main():
    nc = await nats.connect("nats://localhost:4222")
    chat.start()

    async def on_msg(m):
        chat.send(m.data.decode())      # opens a turn, or lands mid-flight
        await m.ack()

    await nc.subscribe("agent.msgs", cb=on_msg)
    await stop_event.wait()


asyncio.run(main())
```

`chat.send` is thread-safe and never blocks, so omni does not care which one you pick.
More in [`examples/`](examples/).

`status` is `idle` before `start`, then `busy` / `waiting`, then `stopped`. `chat.idle`
is the one a polling loop wants: waiting, with nothing left to process. Sending to a
stopped chat raises rather than dropping the message on the floor.

---

## Nothing changes mid-turn

This is the rule the whole design hangs off.

```python
chat.intelligence(9)              # noted now
chat.active_inference_providers(["claude", "openai"])
chat.system_prompt("...")
chat.disable_subagents()
```

Every one of those is recorded when you call it and **applied at the next turn
boundary** — after the running tool finishes and the turn ends. A model never changes
underneath itself.

Calling any of them again overwrites the last value. Same for
`load_or_create_session`: that is how a new session is started.

### Prompts and isolation

```python
chat.system_prompt("...")            # replace the provider's own prompt
chat.system_prompt_file("p.txt")
chat.append_system_prompt("...")     # or keep theirs and add
chat.disable_subagents()             # so only workers you define get used
chat.disable_mcp()                   # no MCP servers, no external connectors
```

---

## Sessions, and how switching works

`~/.omni/sessions/{id}.jsonl` is the source of truth. It is the event log — the same
objects your handlers see, appended in order. It outlives any single provider.

```python
chat = Inference.load_or_create_session("nightly-triage")
```

One live chat per session id. A second attempt raises `SessionBusy`; a lock whose owner
died is reclaimed, so a crash never wedges a session shut.

Alongside it, `{id}.meta.json` remembers each provider's **own** session and how far up
the omni log it has already seen:

```json
{"providers": {"claude": {"id": "3cb0…", "synced": 19},
               "google": {"id": "1dbc…", "synced": 26}}}
```

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

Tool output is capped inside a seed. The log keeps every byte.

---

## Auth

```python
Inference.claude.auth_status          # 'authenticated' | 'unauthenticated'
print(Inference.claude.start_auth())  # the URL to open
Inference.claude.finish_auth("code-or-redirect-url")
```

omni drives each CLI's own login rather than making you use the CLI: it starts the
flow, hands you the URL, and types the code back if one is wanted. Codex runs its own
browser callback, so there `finish_auth` waits rather than types and the code is
ignored. If a CLI does something unexpected, whatever it printed is handed back
verbatim — a confusing message you can read beats a silent failure.

One account per provider.

## Limits

```python
Inference.openai.limits
# {'5h': {'used': 0.0, 'reset': 1787209867}, '7d': {'used': 0.16, 'reset': 1787196805}}
```

`used` is a fraction, `0.16` being 16%. `'unauthenticated'` if you are not signed in.
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

Everything `on_event` sees, plus omni's own bookkeeping: every launch, model change,
provider switch, new session, message in, tool call, error, stop. All of it is the same
`Event` type, so it is parsable without a second schema, and it is the same thing that
is already on disk in the session file.

---

## Per-provider notes

**Claude Code** — flags do the work. Subagents and MCP switch off from the command line
when you ask; slash commands and memory files (`CLAUDE.md`, auto memory, org memory) are
off unconditionally, so a run means the same thing on anyone's machine. Seeding is a file write into
`~/.claude/projects/<slug>/<uuid>.jsonl`; resume then treats it as real history. Model
and effort change over the control channel between turns, so re-tuning does not restart
anything or re-read the conversation.

**Codex** — the app server, not `exec`. It always runs against a `CODEX_HOME` of its own
under `~/.omni/jails/`, holding exactly two things: a symlink to your real `auth.json`
(linked, not copied, so a token refresh is not lost) and a near-empty `config.toml`.
That folder *is* the isolation — codex has nothing left to auto-load, so MCP servers,
hooks and `AGENTS.md` never appear whether or not you asked. Skills live outside
`CODEX_HOME` entirely, so `disable_subagents()` switches them off one at a time over the
protocol. History is seeded with `thread/inject_items`; model and effort are per-turn
parameters, so re-tuning is free.
**Antigravity** — the most restricted. There is no flag for MCP, no flag for subagents,
and no way to seed history, so omni lets it load what it wants and folds prior
conversation into the front of the next message — one turn, not two. `disable_subagents()`
and `disable_mcp()` cannot be honoured here; omni logs a `CONFIG` event saying so rather
than pretending. An injected message runs as its own turn instead of joining the one in
flight, and there is no way to interrupt agy at all. Its cold start is
~10s per launch. An unrecognised conversation id makes agy silently start a new one, so
omni checks the id it gets back and re-seeds from the top if it was not the one it asked
for.

---

## What omni will not do

- **Reasoning is never carried.** It is signed or encrypted per vendor and cannot be
  replayed anywhere else. omni records that thinking happened and moves on.
- **Tools are observed, not defined.** omni does not install tools into a provider or
  rename theirs. Whatever the CLI does, omni reports.
- **One account per provider.** No multi-account support.
- **A switch is a real restart** of the provider process, so it costs the destination a
  context read. Model changes within a provider do not.

---

## Contributing

`omni/` is small on purpose and split the way the problem is:

```
omni/
  events.py        the vocabulary — one type for everything
  chat.py          the engine: one conductor thread, turn boundaries, switching
  inference.py     the front door
  translate.py     history → something a foreign provider can read
  session/         the log, the provider map, the one-owner lock
  intelligence/    the 0-10 dial and its ladder
  providers/       claude/ · openai/ · google/, plus the contract they share
  shared/          paths, jsonl, clock, callback bus, subprocess plumbing
tools/
  build_ladder.py  turns GDPval-AA v2 into the packaged dial
docs/              the site and the registry omni reads its dial from
```

Adding a provider means an `Account` and a `Runner` — see
[`omni/providers/base.py`](omni/providers/base.py), then `providers.register(...)`.
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) explains why the pieces are shaped this way.

```bash
pytest            # fast, no CLI needed
pytest -m live    # drives the real CLIs; needs auth, spends a little quota
```

MIT.
