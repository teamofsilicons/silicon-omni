# Changelog

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
