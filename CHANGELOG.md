# Changelog

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
same intelligence level again over whoever is left — so the chat carries on somewhere
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
