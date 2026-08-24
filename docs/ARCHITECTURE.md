# How omni is put together

The public object is still a `Chat`, but the conversation no longer lives in the
Python process holding it. A persistent Rust daemon owns sessions, provider processes,
history, routing, and turn boundaries. Python, the CLI, and Rust all use the same
socket protocol.

```text
Python client ─┐
omni CLI ──────┼── NDJSON over a Unix socket ── omnid
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
session `snapshot` at that point. Replies and events may be interleaved, and independent
requests on one socket may finish out of order, so every client has one reader loop that
sorts them by id or subscription.

The daemon gives requests for the same session on the same socket one FIFO lane. Account
operations for one provider and test-provider controls have equivalent lanes. Slow
discovery or account work therefore cannot hold up an unrelated session, while an
`open` / `set` / `send` sequence for one session cannot overtake itself. Requests from
separate client sockets meet in the conductor in arrival order; no cross-process total
order is invented.

The protocol is deliberately ordinary. It needs no generated bindings, and a client in
another language only needs a Unix socket, JSON, and a map of requests waiting by id.
Unknown fields are ignored. Unknown operations are refused by name. One request line is
capped at 16 MiB, and at most 128 request tasks run across the daemon at once; socket
readers then apply backpressure instead of creating unbounded allocations or OS threads.
A shutdown closes that global admission gate, rejects new work, and waits for every
request already accepted on every connection before provider cleanup begins.

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
after the latest event.” Replay is not truncated, but each connection has a finite
socket outbox. An actively reading client drains a long replay incrementally; one that
stops reading is disconnected after a bounded wait and can resume from the next
sequence it still needs.

Disconnecting or calling `detach` removes only that listener. Calling `stop` is an
instruction to end the shared session and notify every listener; it is not another word
for disconnect. Clients commit their local stopped/detached state only after the daemon
acknowledges it. A refusal or timeout closes the now-uncertain transport and leaves the
session handle reopenable; losing a socket proves detach but never proves that the
shared session stopped.

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
provider and immediately replace it. Each setting is written into metadata before its
`CONFIG` event is recorded. Provider lists, intelligence, both prompt forms, isolation,
auth-failover policy, and CWD therefore survive cold reaping and daemon restarts.

## One vocabulary, one log

`Event` is the shared schema in Rust, Python, on the wire, and in
`~/.omni/sessions/{id}.jsonl`. The session file is the event log; there is no private
history format to reconcile with it. `HISTORY_TYPES` marks the conversational subset a
provider needs when it arrives late.

`v` selects the on-disk schema (`1` today, also the default for pre-versioned records),
`seq` orders the complete session log, and `turn` groups activity beginning with turn
zero. The optional `native` map retains provider identities needed for exact diagnosis
and replay — such as Claude message/tool IDs, Codex thread/turn/item IDs, and agy
conversation/step IDs. Those values stay in the source log but are not treated as
portable conversation semantics.

Events are coarse on purpose. Provider token deltas are assembled into one `TEXT`
event. `THINKING` records that reasoning happened but carries no reasoning content.

The daemon persists an event before it publishes it. Each JSONL append is data-synced at
that boundary. A valid final JSON record that merely lacks its newline is preserved and
delimited before the next append; an invalid partial tail is ignored on read and
truncated before that append. A malformed complete line, invalid event, sequence gap,
or foreign session id is corruption, not a recoverable torn write, and the existing
file is preserved while that session is refused. Each event frame includes the settled
session snapshot, so Python, a CLI, and a Rust client do not each have to reimplement
the state machine.

Omni-owned directories are forced to mode `0700`. The control socket, daemon PID/log,
session JSONL, and session metadata are `0600`, so a permissive process umask does not
expose prompts, queued messages, tool results, or auth-adjacent state to another local
account. Metadata is replaced via a data-synced `0600` staging file, atomic rename, and
parent-directory sync. A persistence failure is terminal for that live session:
provider work is stopped instead of advertising an event, accepted message, or setting
that was not saved.

## Nothing changes mid-turn

Changing intelligence, providers, prompts, isolation, auth-failover policy, or working
directory queues the setting immediately. The conductor persists it and records a
`CONFIG` event before the running provider sees it at the next turn boundary.

The first client that creates a session offers its current working directory, which is
pinned with the other settings. A later opener's process directory is only a fallback
for a genuinely new session and cannot silently move restored work. An explicit CWD
setting does move it at the next boundary and follows the normal resume/reseed path.

The conductor compares one signature:

```text
(provider, model, effort, config)
```

If only model or effort changed and the adapter can re-tune, the process stays up.
Otherwise the conductor parks or replaces it. A message arriving while a replacement
is pending stays in the outbox instead of being handed to a provider the daemon is
about to leave.

`send` first writes `{id, text}` into a transactional FIFO in metadata. Only after that
replacement and its directory entry are durable does the daemon acknowledge the
caller. The message becomes a `START` or `INJECTED` event only at the adapter's delivery
boundary: direct native acceptance for Codex/agy, or Claude's matching replay echo.
That event retains the public `extra.message_id`; after its JSONL append is durable the
matching metadata entry is removed. A crash in between is reconciled exactly by id,
including when adjacent messages have identical text. A refused delivery stays queued
for retry and is never both seeded and sent.

## Warm providers and switching

`{id}.meta.json` remembers the accepted-message FIFO, durable settings, and each
provider’s native session id plus `synced`, the end of the contiguous omni-log prefix
that native session has seen.

- Continuing on the active provider uses the already-running process.
- Switching away parks a runner when its protocol permits it.
- Returning catches that runner up with history since `synced + 1`.
- If it cannot accept history in place, it is restarted and resumed natively.
- If the provider forgot the native id, omni starts clean and seeds the whole history.

Output already in flight when a runner is replaced keeps that runner's original omni
turn and is persisted with `extra.late=true`. It cannot end the replacement's turn.
Such an event leaves a deliberate hole in the old native session's `synced` watermark;
on return, omni supplies the late or foreign history needed to close the hole while
filtering events that same native session had already produced.

Codex has an additional shared layer: one warm app server per effective flag set serves
many omni sessions. Notifications carry a Codex thread id and are routed to the right
runner. Account and rate-limit notifications are cached on that same connection.

Claude can change model and effort through its control channel, but cannot inject
missed history into a process that is already reading, so a stale parked Claude runner
is restarted. Codex catches up with `thread/inject_items`. Antigravity has no structural
seed operation; missed history is rendered into the front of its next user message.

Delivery is not inferred uniformly across unlike protocols. Claude's exact
`--replay-user-messages` echo confirms which FIFO user line landed; unrelated replay
chatter is ignored. Codex confirms `turn/steer` or `turn/start` directly. Antigravity
accepts a mid-turn write as a distinct next native turn. The conductor records
`START`/`INJECTED` only at the corresponding provider boundary and keeps native IDs on
the resulting events.

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
- Process stop has a bounded pipe-reader handoff and closes its callback gate before it
  returns. A small death-pipe guardian owns each provider and probe process group, so an
  abrupt daemon exit kills it even when Rust destructors cannot run.
- A late event from the provider just left is logged but cannot close its successor’s
  turn; it retains its origin turn and holds that provider's contiguous sync watermark
  open until the history is supplied on return.
- Explicitly stopping a session lets a real provider `END` win; if none arrives, omni
  durably records one synthetic `END` marked `interrupted` and `stopped`. Accepted but
  undelivered messages remain in metadata rather than being erased.
- Provider request waiters are released when their underlying process dies.
- Browser-login children expire, are replaced cleanly, and are reaped on daemon
  shutdown.
- `SIGTERM` and `SIGINT` close request admission, drain accepted work, wake the accept
  loop, and take the same registry/login/provider cleanup path as protocol shutdown.
- Existing malformed metadata or complete-line log corruption is preserved for
  diagnosis and refuses provider launch instead of silently starting over.

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

## Distribution

The platform Python wheel contains both Rust executables: `omnid` and the native `omni`
terminal client. Its Python console-script entry point only replaces itself with that
bundled `omni`, preserving arguments and environment. The same binaries remain
independently installable as the `omni-daemon` and `silicon-omni-cli` Cargo packages; Rust
programs use `omni-client` directly.

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
crates/omni-client/
  lib.rs              synchronous calls, reply multiplexing, and frame subscriptions
crates/omni-cli/
  main.rs             daemon/session/account commands and streaming terminal chat
omni/
  client/             daemon discovery/startup and Python socket transport
  chat.py             callbacks and Python session handle
  inference.py        public entry point and account handles
  events.py           Python view of the wire event
```

Run `cargo test --workspace` for the core and daemon, then `pytest` for the public
Python API through a real private-home daemon. `pytest -m live` is intentionally
separate: it drives installed vendor CLIs, spends quota, and uses the real `~/.omni`.
