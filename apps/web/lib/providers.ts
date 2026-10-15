/**
 * The three CLIs omni drives, and what each accepts.
 *
 * Effort is a per-provider thing, not a per-model thing: every model a given
 * CLI can run takes the same effort words, so these are hard-coded rather than
 * looked up. An empty effort means the flag is not passed at all, which some
 * models require and which Antigravity needs whenever the slug already carries
 * the effort (`gemini-3.8-flash-high`).
 */

export type ProviderId = "claude-code-cli" | "codex-app-server" | "antigravity-cli"

export interface Provider {
  id: ProviderId
  label: string
  cli: string
  /** litellm's own provider ids for the models this CLI usually serves. */
  vendors: string[]
  /** every effort string the CLI accepts, weakest first. "" means omit the flag. */
  efforts: string[]
  /** how the model is actually run, with <model> and <effort> filled in */
  run: string
  /** what kind of thing `run` is. Two of the three are shell; one is not, and
      printing a JSON-RPC call in a <code> block as if you could paste it into a
      terminal is a small lie the site used to tell on its own landing page. */
  wire: "a shell command" | "a JSON-RPC call over stdio"
  /** the effort argument on its own, dropped entirely when effort is "" */
  effortArg: string
  /**
   * How this CLI is told to run hot, if it can be.
   *
   * Codex takes a config override: `service_tier = "fast"` lives in
   * ~/.codex/config.toml and `-c service_tier="fast"` overrides it per run.
   *
   * Claude Code has fast mode — the binary carries `fastMode`,
   * `fastModePerSessionOptIn` and `AnthropicBetaFastMode` — but exposes no
   * `--fast` flag. It is a per-session opt-in, toggled with `/fast`, and only
   * on Opus 5 and 4.8. So the registry says fast and the library arranges it;
   * what is printed here is the session setting, not a flag to paste.
   *
   * Antigravity has no such thing, and `fast` on a Google pick is ignored
   * rather than refused.
   */
  fastArg: string | null
}

export const PROVIDERS: Provider[] = [
  {
    id: "claude-code-cli",
    label: "claude-code-cli",
    cli: "claude -p",
    run: "claude -p --model <model> <effort><fast>",
    wire: "a shell command",
    effortArg: "--effort <effort>",
    fastArg: "  ·  /fast",
    vendors: ["anthropic"],
    efforts: ["", "low", "medium", "high", "xhigh", "max"],
  },
  {
    id: "codex-app-server",
    label: "codex-app-server",
    cli: "codex app-server",
    run: 'codex app-server --stdio<fast>  ·  turn/start { model: "<model>"<effort> }',
    wire: "a JSON-RPC call over stdio",
    effortArg: ', effort: "<effort>"',
    fastArg: ' -c service_tier="fast"',
    vendors: ["openai"],
    efforts: ["", "low", "medium", "high", "xhigh", "max", "ultra"],
  },
  {
    id: "antigravity-cli",
    label: "antigravity-cli",
    cli: "agy",
    run: "agy --model <model> <effort> --print<fast>",
    wire: "a shell command",
    effortArg: "--effort <effort>",
    fastArg: null,
    vendors: ["gemini", "anthropic", "openai"],
    efforts: ["", "low", "medium", "high"],
  },
]

/**
 * The rung, written out as the command it is.
 *
 * The point of showing this is that there is nothing else to know: the strings
 * in the dial go to the CLI untouched, so whatever a future model is called,
 * this is how it gets run.
 *
 * Two of the three are literally a shell command. Codex is not — it is a method
 * on a JSON-RPC session — so `wire` says which, and callers are expected to
 * print it rather than imply a terminal.
 */
export function invocation(rung: {
  provider: string
  model: string
  effort: string
  fast?: boolean
}): string {
  const known = provider(rung.provider)
  if (!known) return ""
  const effort = rung.effort ? known.effortArg.replace("<effort>", rung.effort) : ""
  // asked for fast on a CLI that has no such thing: ignored, not refused
  const fast = rung.fast && known.fastArg ? known.fastArg : ""
  return known.run
    .replace("<model>", rung.model)
    .replace("<effort>", effort)
    .replace("<fast>", fast)
    .replace(/\s+/g, " ")
    .trim()
}

/** Whether asking this provider to run hot means anything at all. */
export function canRunFast(id: string): boolean {
  return Boolean(provider(id)?.fastArg)
}

export function provider(id: string): Provider | undefined {
  return PROVIDERS.find((p) => p.id === id)
}

/** Every non-empty combination of providers, named the way omni asks for them. */
export function combinations(): string[] {
  const ids = PROVIDERS.map((p) => p.id)
  const out: string[] = []
  for (let mask = 1; mask < 1 << ids.length; mask++) {
    out.push(ids.filter((_, i) => mask & (1 << i)).sort().join("+"))
  }
  return out
}

/** One dial per set of providers, named the same way on both sides. */
export function key(providers: string[]): string {
  return [...new Set(providers)].sort().join("+")
}
