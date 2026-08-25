# Changelog

## 0.7.0

**There are three ways to say what should answer, and `chat.model()` takes all
of them.** A word — `chat.model("code")` — is a shortlist somebody chose, best
first, and the first vendor on it you are signed into answers. A number —
`chat.model(intelligence=7)` — is the dial, still the left edge of a board where
a model earns a rung when nothing else is both better and cheaper. A model —
`chat.model(model="gemini-3.7-flash-low", provider="google")` — is you already
knowing, passed to the CLI verbatim and resolved on the machine with no network
at all. Saying two of them in one call is refused rather than resolved by
precedence.

**0.6's weighted selector is gone.** It scored every model on three normalised
axes and took a weighted sum, and it kept producing answers that were defensible
and wrong: three models within one percent of each other and a hundred Elo
apart, and a winner that moved when a normalisation constant did. Ranking is not
judgement, so the judgement is written down instead, in the registry, by hand.

**`fast` asks a CLI for its faster tier.** For Codex it is a launch flag,
`-c service_tier="fast"` — and because a flag set is what a shared app-server is
keyed on, a hot chat rents its own warm server while a normal one is untouched
by it. Claude Code has a fast mode on its largest models, opted into per session.
Antigravity has no such thing, and asking for it there is ignored rather than
refused.

**The registry endpoint moved to `/choose.json`,** and the cache with it. The
old `/intelligence.json` no longer exists, so 0.6 and earlier cannot resolve a
model at all; they fail with `NoAnswer` rather than guessing.

**Removed, rather than deprecated.** `Chat.inteligence()`, the misspelled alias.
`Chat.intelligence()` itself, folded into `Chat.model()`. The `level` wire key
and every translation of it. The CLI aliases `--level`, `--from`, `events` and
`attach`. `NoDial` is now `NoAnswer`. The `model` getter is `running_model`,
because `model` is the setter now — in Python and in Rust.

## 0.5.0

**Python, Rust, and the terminal now share one public vocabulary.** `Inference`
opens a `Chat`; the session id names its durable conversation; `intelligence` is
the 0–10 setting; and every occurrence is an `Event`. Python and JSON use
`event.type`, while Rust uses `event.event_type` because `type` is reserved;
`event.kind` now means only an error classification. Persisted reads are
`history`, replay begins at `since`, and the lifecycle is consistently
`start` / `send` / `detach` / `stop` / `refresh`. The CLI follows the same terms
with `--intelligence`, `--since`, `history`, and `logs`; the 0.4 spellings
`--level`, `--from`, `events`, and `attach` remain accepted as compatibility
aliases. The version-1 wire and old metadata spellings remain readable without
being exposed as a second public API.

**Rust now has the same high-level `Inference → Chat → Event` path as Python.**
Transport-oriented `Client`, `OpenOptions`, `Session`, `Frame`, and `Request`
remain available for low-level integrations, but they are no longer the primary
documented API.

**Steady-state provider turns are the performance contract.** Claude, Codex, and
Antigravity each keep one live runner and native conversation across settled,
consecutive turns. The release suite now proves five turns produce five distinct
START/END pairs without injection or process replacement, and the grouped live
benchmark teaches five facts to each provider before Claude recalls all fifteen.

**The public Rust package is `silicon-omni`.** Rust users add the same product name
with Cargo and import it as `silicon_omni`; the unpublished internal `omni-client`
package name has been retired before the first crates.io release.

**The terminal client is `silicon-omni`, with `so` as its short name.** Both the
Cargo-installed CLI and Python wheel expose those commands over the same native
client. The older experimental `omni` executable name is retired in 0.5.0.

**Antigravity now survives a return with unchanged tuning.** A parked provider whose
model and effort already match is adopted directly. Omni no longer asks a
non-retunable Antigravity process to perform a no-op retune and then cold-starts it
when that unsupported request returns false. Real tuning changes still restart a
provider that cannot apply them live.

**The durable event hot path does less repeated filesystem work.** A live session owns
one validated JSONL appender instead of reopening, chmodding, seeking, and checking
the log tail for every event. Every append still crosses the same `sync_data`
persist-before-publish boundary; torn-tail repair, private permissions, corruption
refusal, and sequence rollback semantics are unchanged. Repeated metadata values no
longer replace an already-identical durable file.

## 0.4.0

**The conversation moved into a Rust daemon.** `omnid` now owns provider processes,
turn boundaries, history, switching, account probes, and the intelligence cache. The
Python package is a thin Unix-socket client and starts the daemon automatically on first
use. The public `Inference`, `Chat`, and `Event` API remains the Python entry point.

**Rust and terminal clients use that same daemon.** The synchronous `omni-client`
crate multiplexes concurrent calls and session event subscriptions over one socket.
Independent replies may complete out of order while requests for one session retain
FIFO order. The new `omni` command covers streaming chat, attach/replay, sessions,
settings, providers, the intelligence dial, accounts, raw protocol calls, and daemon
lifecycle. Platform Python wheels contain both `omni` and `omnid`; the Cargo packages
remain independently installable. The terminal crate is published as
`silicon-omni-cli` because the unrelated `omni-cli` name is already occupied.

**Native distributions are exercised as distributions.** CI builds audited
manylinux 2.28 wheels for x86_64/aarch64 and macOS wheels for Intel/Apple silicon,
using the workspace's Rust 1.85 minimum. Each platform produces one `py3-none` wheel,
installs it outside the source tree, and smoke-tests both `omni` and the bundled
`omnid`; the sdist is built, checked, and inspected separately.

**Sessions survive their client.** Detaching or exiting leaves the provider warm for a
15-minute grace period. Reopening the same id reconnects to the live conversation rather
than launching a second provider and rebuilding its context.

**Several clients may attach to one session.** Each receives the same ordered events,
and any may send. A client can replay from a sequence number or request only future
events. `SessionBusy` remains importable for compatibility but is no longer raised;
`stop()` ends the shared session, while `detach()` only removes that client.

**Reconnect and lifecycle calls have explicit commit points.** Python generations
isolate stale socket frames from a reopened chat without waiting for slow user
callbacks, and replay cursors advance only for frames that generation accepted. Python
and Rust clients mark `stop()` or `detach()` complete only after the daemon reply;
refusal or timeout closes the uncertain link, surfaces the error, and leaves a failed
stop reopenable for retry.

**Daemon concurrency is bounded and shutdown is global.** Replay uses a finite socket
outbox and disconnects a stalled reader without blocking a session. Request lines are
capped at 16 MiB and 128 request tasks run at once; further socket reads provide
backpressure. Shutdown closes admission and drains already accepted work across every
client before cleanup. Session lifecycle gates serialize open/send/set/stop/reap, and
cold reaping revalidates that an unobserved session is still idle before stopping it.

**Session configuration is durable.** Active providers, intelligence, prompts,
subagent/MCP isolation, auth-failover policy, and working directory live in metadata and
survive cold reaping or daemon restart. A new session adopts the opening client's CWD;
later clients cannot move it accidentally, while an explicit CWD setting still applies
at the next turn boundary.

**Events are versioned and retain native identity.** New records carry schema `v=1`,
and old records without `v` load as version 1. `turn` groups cross-provider activity,
while the optional `native` map keeps provider conversation, message, turn, item, tool,
and step IDs without making them portable history. Private runner epochs are stripped
before persistence; a public `extra.message_id` correlates durable send acceptance with
provider delivery, and output from a replaced runner retains its origin turn with
`extra.late=true` instead of acting on its successor.

**The source of truth has a real durability boundary.** Session JSONL is data-synced
before an event is published. A valid unterminated final record is preserved and an
invalid partial tail is repaired before append, while complete-line corruption,
invalid events, sequence gaps, and foreign session records are preserved and refuse
the session instead of being skipped. Accepted sends first enter a transactionally
rewritten metadata FIFO and are acknowledged only after it is durable; correlated
`START`/`INJECTED` events make crash reconciliation exact. Explicit stop durably closes
an open turn once, synthesizing an interrupted `END` only when the provider did not
emit one. Omni-owned directories are private (`0700`); its socket, PID/log, JSONL, and
metadata files are forced to `0600`, with new directory entries synced too.

**Provider processes stay hot across switches where their protocols allow it.** Codex
threads share a warm app server and catch up with injected history. Antigravity carries
missed history into its next message. Claude is restarted only when it cannot be caught
up in place. Model and effort changes still re-tune without a restart when supported.

**Provider edge cases now follow the native protocols.** Claude uses only the exact
replayed-user FIFO as delivery acknowledgement and preserves unknown structured output.
Codex propagates shared app-server exits, retries incomplete skill isolation, persists a
first login through its jail symlink, and cleanly reseeds a forgotten thread.
Antigravity requires a reported conversation id, keeps replacement and appended prompts
in order, treats a mid-turn message as the next native turn, and leaves missing quota as
unknown rather than zero used. Failure classification now matches whole tokens and
phrases instead of fragments inside unrelated words. Browser-login children expire and
are reaped on shutdown. Provider/probe process groups have death-pipe guardians for
abrupt daemon loss, process stop cannot hang on inherited pipes or call back afterward,
and `SIGTERM`/`SIGINT` take the same graceful cleanup path as protocol shutdown.

**The test provider crosses the real transport.** Python tests drive the shipped Rust
double through the same daemon and socket used by live providers. Rust tests cover the
conductor and captured provider protocols directly.

## 0.3.0

**One test provider, not two.** `omni.providers.test` absorbs everything the private
double under `tests/` could do, because all of it is useful to anyone testing their own
code against omni — not just to omni's own suite:

```python
test.install("alpha", "beta")            # two of them, so you can test a switch
test.running("alpha").autoreply = False  # hold the turn open
chat.send("hello")
test.running("alpha").fail("auth")       # now lose the login
```

`install()` takes provider names and an optional set of rungs; `running()` hands you the
live runner, which records `given` and `sent`. The knobs each mimic a real CLI: `defer`
is agy only seeing history with the next message, `tunable = False` is agy being unable
to change model without a restart, and a native id starting with `gone-` is any provider
that has forgotten a session omni thinks it still has.

**A provider says what it cannot do once.** An `unsupported` notice now fires when you
set the thing and when the conversation arrives on that provider — not on every relaunch
underneath an unchanged conversation.

**Codex says that `enable_mcp()` will not reach it.** CODEX_HOME is redirected whether or
not you asked, so opting back into MCP does not get you MCP. It now logs that rather than
letting you believe your servers are loaded.

**`used` and `reset` may be `None`.** Documented and tested across all three providers:
some plans report no windows, and *nobody said* is not the same as *nothing spent*.

## 0.2.0

**Breaking: a chat now starts quiet.** `disable_subagents` and `disable_mcp` both
default to on. A provider that brings its own subagents and MCP servers makes the same
run mean different things on different machines, so you opt back in rather than out:

```python
chat.enable_subagents()   # new
chat.enable_mcp()         # new
```

If you were relying on the old permissive default, add those two lines. `disable_*()`
still exists and is now a no-op that reads as intent. Codex's `project_doc_max_bytes=0`
is applied on every launch, so opting into subagents cannot bring an `AGENTS.md` with
them.

**A lost login costs the provider, not the conversation.** An `ERROR`/`auth` mid-run
drops that provider from the chat, emits `CONFIG`/`provider_removed`, and resolves the
same intelligence value again over whoever is left — so the chat carries on somewhere
else. Off with `chat.disable_autoremoving_unauthenticated_providers()`.

**Every `reset` is an RFC3339 UTC string**, from all three providers, whatever they
answer in natively.

**Seeds are whole.** The 2 000-character cap on tool output and the 400-character cap on
tool arguments are gone. A provider arriving late gets the entire conversation.

**New: a provider that needs no CLI.** `omni.providers.test` is deterministic, needs no
login and no quota, and pins its own 0-10 dial. Nothing registers it but an explicit
`install()`.

**Fixed.** A dying CLI reports its failure and the end of its turn in one breath; the
straggling event is no longer applied to the provider that has just replaced it, which
could close a turn that was still open.

## 0.1.0

First release.
