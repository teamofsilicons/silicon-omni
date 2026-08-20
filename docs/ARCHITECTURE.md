# How omni is put together

This is the reasoning behind the shapes. If you are here to add a provider, skim
"The two objects" and "Seeding" and you have what you need.

## One vocabulary, one log

`Event` is the only shape in the system. The same objects go to `@chat.on_event`,
to `@chat.logs`, and into `~/.omni/sessions/{id}.jsonl` — so the session file *is*
the event log and there is never a second schema to keep in sync.

That is why events are coarse. `TEXT` is a completed assistant message, not a token,
because a token stream is not something you want to store or replay. Every provider
streams deltas; omni stitches them and emits once.

`HISTORY_TYPES` marks the events that are conversation rather than bookkeeping. That
distinction is the only thing separating "the log" from "what a provider gets replayed".

## One thread

Provider output, user messages, and lifecycle changes all land on one queue, handled
by one conductor thread (`Chat.run`). Nothing else mutates chat state.

This buys three things worth more than the concurrency it gives up:

- callbacks fire one at a time, in the order things happened;
- `send()` is safe from any thread, including from inside a handler;
- tearing down a provider never races the reader thread that is still draining it.

Reader threads only ever call `Chat.push`, which enqueues.

## Nothing changes mid-turn

`chat.intelligence(9)` does not restart anything. It records the wish. At the next
`END`, `rebuild()` compares what is wanted against what is running and acts.

The comparison is a plain tuple — `(provider, model, effort, config)` — so "has
anything changed" is one `!=` and there is no separate pending-changes bookkeeping to
get out of step.

A message that arrives while a rebuild is pending is held in `outbox` rather than
handed to a provider that is about to be replaced. And `dispatch` rebuilds *before*
recording the message, so a message is either seeded into a provider or sent to it,
never both. Both of those were bugs first.

## Two objects per provider

`Account` is global and session-free: is the CLI here, are we signed in, how much
quota is left, how do we log in. It is what `Inference.claude` gives you.

`Runner` is one live process driving one session. It runs turns and reports events.
It does not own history, session identity, or model choice.

Everything portable lives above them, which is why the adapters are thin and why
`tests/fake.py` can exercise the whole engine without a CLI in sight.

Adapters are imported lazily. Importing omni should not drag in three protocols.

## Seeding, and the `synced` mark

`{id}.meta.json` holds, per provider, its native session id and `synced`: how far up
the omni log that session has already seen.

- `synced` is advanced at the end of each turn, on the provider that ran it.
- It is **not** advanced when switching away. Everything recorded since that
  provider's last turn is precisely what it has to be told on the way back.

So arriving somewhere replays `history(since=synced + 1)` and nothing more. A provider
that no longer recognises its session (agy silently forks; a Claude file can be
deleted) reports back a different id than it was asked for, and `launch()` notices,
throws the half-seeded runner away, and starts clean with the whole story.

## Translation

`translate.transcript()` turns history into `{role, text}` turns. Tool activity from
elsewhere becomes `[Tool: args]` / `[Tool result: …]`.

The reason this is not lossy: seeding only happens on a switch, so the history being
handed over is by definition history the destination did not live through. Faking
structured tool calls it never made would be a lie the API can reject. Rendering them
as text is honest, and the omni log keeps the structured original — going back to the
provider that made the call replays *its own* session, where the call is real.

`flatten()` is the same thing as one message, for providers that accept nothing else.

## Re-tuning instead of restarting

A switch restarts a process, which costs the destination a context read. A *model*
change within one provider does not have to.

`Runner.retune(model, effort)` returns `True` if the running process took it. Claude
does it over its stdin control channel; Codex takes model and effort per turn, so it
costs nothing at all; agy cannot, returns `False`, and omni restarts it. `Chat.tunable`
only offers a retune when provider and config are otherwise identical.

## When things die

The happy path is the easy half. Every one of these was a bug first, and each has a
test:

**A message is recorded only once a runner has taken it.** `flush()` peeks at the
outbox, calls `send`, records, *then* pops. A send that raises leaves the message
queued and nothing written, so a retry delivers it exactly once. It also means a
message can never be both seeded into a provider and sent to it.

**A provider that will not start does not swallow anything.** `attempt()` wraps
`rebuild()`; a failure records an `ERROR` and drops back to `waiting` with the message
still queued. Hanging in `busy` forever would be worse than saying so.

**A crash closes its turn.** `collapse()` marks the dying provider synced up to what it
actually saw, stops it (dropping the reference alone would orphan a live CLI), and
records a synthetic `END` so anything watching for turn boundaries — including the
caller's own loop — is not left waiting for one that will never come.

**stderr is not a crash.** Only `exited()` reports `kind="crash"`. A line that merely
contains the word "error" gets `kind="stderr"`; treating it as a death used to tear down
a perfectly healthy process mid-turn.

**A provider is not marked caught up until it has the history.** `Runner.seeded` is
`False` on agy until the seed actually goes out with a message. Marking it early meant a
runner replaced before that point skipped that history forever.

**Waits end when the thing being waited on dies.** agy's `exited()` sets its ready flag
so a dead CLI does not burn the full cold-start timeout, and `AppServer.closed()` fails
every pending request rather than letting callers sit on a 120s timeout.

**Files are written whole or not at all.** `Meta.save()` writes to a temp file and
`os.replace`s it, and a corrupt read starts over instead of raising. It is rewritten on
every turn, so a kill lands mid-write eventually.

**The lock has exactly one winner.** Taking over a dead owner's lock does not unlink and
re-create — two contenders would both succeed. It writes its claim over the old one and
reads it back; only the process that sees its own pid holds it.

## The dial is data, not a calculation

`intelligence/registry.py` does not know what a Pareto frontier is. It looks up a
finished map from level to model, keyed by the set of providers you have, and that is
all. The map is fetched from upstream, cached under `~/.omni/cache` for an hour, and
falls back to the packaged `ladder.json`.

That split is deliberate. Deciding which models belong on the dial needs a benchmark
leaderboard, per-model measured costs, and a hand-checked mapping from CLI slugs to
leaderboard rows — none of which should be in a client, and all of which will move to
the remote registry. `tools/build_ladder.py` does it offline and ships the answer.

The one thing worth understanding about that answer: it is the *left edge* of a
score-versus-price graph, so every step down the dial is genuinely cheaper. Levels can
share a rung when the edge is shorter than eleven points, which is honest — it means
there is nothing in between that anybody should pick.

## Testing

`tests/fake.py` is a provider that is not one: it records what it was seeded with and
what it was sent, and finishes turns exactly when a test says so. Everything about
turns, boundaries, injection, switching and seeding is tested through it, offline.

Adapter tests use lines captured verbatim from real CLI runs. `pytest -m live` runs the
real thing.
