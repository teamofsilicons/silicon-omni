# How omni is put together

The public object is still a `Chat`, but the conversation no longer lives in the
Python process holding it. A persistent Rust daemon owns sessions, provider processes,
history, routing, and turn boundaries. Python is a socket client; the CLI and Rust
client will speak the same protocol.

```text
Python client ─┐
future CLI ────┼── NDJSON over a Unix socket ── omnid
Rust client ───┘                               ├─ session conductors
                                              ├─ event logs and metadata
                                              └─ Claude / Codex / agy processes
```

There is one daemon per `OMNI_HOME`. The first client starts it detached and waits for
`~/.omni/omnid.sock`; later clients connect to that socket. The daemon keeps a session
warm for 15 minutes after its last listener leaves, then stops its providers and drops
the in-memory state. Its JSONL and metadata remain on disk.

## One protocol

The wire format is newline-delimited JSON. A request has an integer `id` and an `op`;
its reply has the same `id`. Event frames have `stream`, `session`, `event`, and the
session `snapshot` at that point. Replies and events may be interleaved, so every client
has one reader loop that sorts them.

The protocol is deliberately ordinary. It needs no generated bindings, and a client in
another language only needs a Unix socket, JSON, and a map of requests waiting by id.
Unknown fields are ignored. Unknown operations are refused by name.

Each open Python `Chat` uses its own connection because the connection is the
subscription. A separate shared connection handles one-off questions such as provider
availability, account status, quota, and the intelligence dial.

## Many clients, one session

The daemon registry holds at most one conductor for a session id. Several connections
may attach to it, every listener receives the same event stream, and any connection may
send or change settings.

An attach includes `from`, the first event sequence number the client wants. The
registry holds the listener lock while it replays the on-disk log and subscribes the
connection, so a live event cannot slip into the seam. A negative value means “start
after the latest event.”

Disconnecting or calling `detach` removes only that listener. Calling `stop` is an
instruction to end the shared session and notify every listener; it is not another word
for disconnect.

## One conductor per session

Provider output, user messages, settings, launch requests, and shutdown all land on one
Rust channel. One conductor thread owns the mutable state and handles those wakes in
order. Other threads hold a cloneable `Handle` that can only post a wake and read an
atomic snapshot.

This buys the invariants the engine cares about:

- events are persisted and published in one order;
- sends from several clients cannot mutate the session concurrently;
- provider teardown cannot race provider output handling;
- settings are applied only at turn boundaries.

Settings supplied before Python calls `start()` travel inside the `open` request. The
daemon queues them before the conductor’s first launch, so it does not start the old
provider and immediately replace it.

## One vocabulary, one log

`Event` is the shared schema in Rust, Python, on the wire, and in
`~/.omni/sessions/{id}.jsonl`. The session file is the event log; there is no private
history format to reconcile with it. `HISTORY_TYPES` marks the conversational subset a
provider needs when it arrives late.

Events are coarse on purpose. Provider token deltas are assembled into one `TEXT`
event. `THINKING` records that reasoning happened but carries no reasoning content.

The daemon persists an event before it publishes it. Each event frame includes the
settled session snapshot, so Python, a CLI, and a Rust client do not each have to
reimplement the state machine.

## Nothing changes mid-turn

Changing intelligence, providers, prompts, isolation, or working directory records the
new setting immediately. The running provider sees it at the next turn boundary.

The conductor compares one signature:

```text
(provider, model, effort, config)
```

If only model or effort changed and the adapter can re-tune, the process stays up.
Otherwise the conductor parks or replaces it. A message arriving while a replacement
is pending stays in the outbox instead of being handed to a provider the daemon is
about to leave.

A message is recorded only after the runner accepts it: peek, send, append, pop. A
failed send therefore remains queued and is never both seeded and sent.

## Warm providers and switching

`{id}.meta.json` remembers each provider’s native session id and `synced`, the last omni
sequence that native session has seen.

- Continuing on the active provider uses the already-running process.
- Switching away parks a runner when its protocol permits it.
- Returning catches that runner up with history since `synced + 1`.
- If it cannot accept history in place, it is restarted and resumed natively.
- If the provider forgot the native id, omni starts clean and seeds the whole history.

Codex has an additional shared layer: one warm app server per effective flag set serves
many omni sessions. Notifications carry a Codex thread id and are routed to the right
runner. Account and rate-limit notifications are cached on that same connection.

Claude can change model and effort through its control channel, but cannot inject
missed history into a process that is already reading, so a stale parked Claude runner
is restarted. Codex catches up with `thread/inject_items`. Antigravity has no structural
seed operation; missed history is rendered into the front of its next user message.

## Translation

`translate` turns history into `{role, text}` turns. A foreign provider’s tool activity
is rendered as readable text rather than forged as a structured call the destination
never made. The JSONL log retains the structured original.

Reasoning is never translated. It is provider-specific, signed or encrypted, and is not
portable conversation state.

## When things die

Failure behavior is part of the architecture, not cleanup after the happy path:

- A runner that refuses a message leaves it queued and records an error.
- A provider that will not launch returns the session to `waiting`; it never hangs in
  `busy`.
- A crash or lost login closes an open turn with a synthetic `END`.
- By default a provider that loses authentication is removed and the same intelligence
  level is resolved over those remaining.
- A dead runner is stopped before its reference is discarded, so no CLI is orphaned.
- A late event from the provider just left is logged but cannot close its successor’s
  turn.
- Provider request waiters are released when their underlying process dies.
- Metadata is written through a staging file and rename; a malformed read starts over
  instead of bricking a session.

## Provider boundary

`Account` is global and session-free: installed state, authentication, login, and quota.
`Runner` drives one provider session: start, send, re-tune, catch up, interrupt, stop,
and emit events. Everything portable lives above those two traits.

Adapters are under `crates/omni-core/src/providers/`. The shipped `test` provider uses
the same traits and conductor as the real three, and the daemon exposes controls for it
so tests in any client language exercise the real socket path.

## The dial is somebody else’s problem

The core contains no model names and performs no ranking. It asks
`omni.teamofsilicons.com/intelligence.json` for a finished map keyed by the available
provider set, and caches that answer under `OMNI_HOME` for an hour. An expired answer is
better than no answer; a machine that has never reached the registry refuses rather
than guessing.

## Repository map

```text
crates/omni-core/
  chat.rs             conductor and turn-boundary rules
  events.rs           canonical event vocabulary
  session/            JSONL store and provider sync metadata
  providers/          Account and Runner traits plus four adapters
  intelligence.rs     remote dial lookup and cache
  translate.rs        portable history rendering
  wire.rs             request, reply, frame, and protocol version
crates/omni-daemon/
  registry.rs         live sessions, listeners, replay, idle reaping
  serve.rs            request operations
  wiring.rs           non-blocking connection writers and subscriptions
omni/
  client/             daemon discovery/startup and Python socket transport
  chat.py             callbacks and Python session handle
  inference.py        public entry point and account handles
  events.py           Python view of the wire event
```

Run `cargo test --workspace` for the core and daemon, then `pytest` for the public
Python API through a real private-home daemon. `pytest -m live` is intentionally
separate: it drives installed vendor CLIs, spends quota, and uses the real `~/.omni`.
