# Public terminology

Python, Rust, and `silicon-omni` / `so` present the same product. They use the
same names even though the daemon's version-1 protocol and older clients still
contain a few compact legacy spellings.

| concept | Python | Rust | terminal |
|---|---|---|---|
| front door | `Inference` | `Inference` | `silicon-omni` / `so` |
| conversation handle | `Chat` | `Chat` | `SESSION` argument |
| durable identity | session id | session id | `SESSION` argument |
| routing dial | `chat.intelligence(n)` | `chat.intelligence(n)` | `--intelligence N` / `set … intelligence N` |
| one occurrence | `Event` | `Event` | an Event; JSON field `type` |
| occurrence discriminator | `event.type` | `event.event_type` | `type` in JSON |
| error classification | `event.kind` | `event.kind` | `kind` in JSON |
| persisted query | `chat.history(since)` | `chat.history_since(since)` | `history --since SEQ` |
| live delivery | `@chat.on_event` | `chat.events()` | streaming command output |
| complete live log | `@chat.logs` | `chat.logs()` | `logs --since SEQ` |
| replay cursor | `since` | `since` | `--since SEQ` |

Session lists, `refresh`, `status`, and streamed snapshots expose the current
`intelligence`, `provider`, `model`, and `effort` with those exact names.

`type` and `kind` are deliberately different words. `type` says what happened,
such as `text`, `tool.call`, or `error`. `kind` is populated on an `error` Event
to say what class of failure it was, such as `auth`, `limit`, or `crash`. Rust
uses `event_type` only because `type` is a reserved word; serialized Rust Events
still use the JSON field `type`.

Likewise, a `Chat` is a client-side handle, while a session is the durable
conversation identity owned by the daemon. Several `Chat` handles can attach to
one session, and a session survives all of them detaching.

## Lifecycle

The lifecycle verbs have the same meaning everywhere:

- `start` attaches the Chat and begins live delivery. Python's `start(since=…)`
  and Rust's `start_since(since)` request replay from that cursor. `since=0`
  includes the complete log; `since=-1` requests only future Events.
- `send` durably submits a user message. It opens a turn or injects into the
  current one; it does not wait for inference to finish.
- `detach` removes only this client. The session and its provider stay warm
  until the daemon's idle timeout.
- `stop` ends the shared session for every attached client and shuts its
  providers down.
- `refresh` asks the daemon for the latest Chat snapshot instead of relying on
  the last streamed snapshot.

## History, Events, and logs

`history` is a finite read of persisted Events. Live event delivery is the
ordered stream of Events emitted after attachment. `logs` observes that same
complete daemon Event stream; Python additionally sends its log handlers a
local `ERROR` Event of kind `handler` when one of that Python client's callbacks
raises. Those local handler errors are not persisted and have no sequence
number.

The CLI's `--json` streaming output is one serialized Event per line. Use
`--frames` only when transport envelopes and session snapshots are needed for a
low-level integration.

## Compatibility names

Version 0.5 accepts the 0.4 terminal spellings `--level`, `--from`, `events`, and
`attach` as compatibility aliases for `--intelligence`, `--since`, `history`,
and `logs`. The version-1 daemon protocol and metadata continue to encode `level`
and `from` for compatibility. They are compatibility inputs and storage keys,
not alternate public terms; new code and all public output use `intelligence`
and `since`.
