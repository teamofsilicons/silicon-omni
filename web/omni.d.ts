/**
 * omni, from a browser. The same vocabulary as the Python and Rust clients.
 */

export declare const BASE_PORT: 1998
export declare const FLOOR_PORT: 1989
export declare const STORAGE_KEY: "omni.web"

export type EventType =
  | "start"
  | "text"
  | "thinking"
  | "tool.call"
  | "tool.result"
  | "end"
  | "injected"
  | "error"
  | "switch_provider"
  | "new_session"
  | "config"

export declare const Event: {
  readonly START: "start"
  readonly TEXT: "text"
  readonly THINKING: "thinking"
  readonly TOOL: { readonly CALL: "tool.call"; readonly RESULT: "tool.result" }
  readonly TOOL_CALL: "tool.call"
  readonly TOOL_RESULT: "tool.result"
  readonly END: "end"
  readonly INJECTED: "injected"
  readonly ERROR: "error"
  readonly SWITCH_PROVIDER: "switch_provider"
  readonly NEW_SESSION: "new_session"
  readonly CONFIG: "config"
}

/** How an ERROR is classified, when it came from a provider. */
export declare const Kind: {
  readonly AUTH: "auth"
  readonly LIMIT: "limit"
  readonly UNAVAILABLE: "unavailable"
  readonly CRASH: "crash"
}

/** One thing that happened. Which fields are set depends on `type`. */
export interface OmniEvent {
  v: number
  type: EventType
  session: string
  seq: number
  at: string
  turn?: number
  provider?: string
  model?: string
  /** START, INJECTED, TEXT, CONFIG */
  text?: string
  /** TOOL.CALL, TOOL.RESULT */
  tool?: string
  id?: string
  args?: Record<string, unknown>
  result?: unknown
  ok?: boolean
  /** ERROR */
  error?: string
  kind?: string
  extra?: Record<string, unknown>
}

/** Where a session stands, as of the event it rode in with. */
export interface Snapshot {
  session: string
  status: "busy" | "waiting" | "stopped" | string
  ask: Ask
  providers: string[]
  provider: string
  model: string
  effort: string
  cwd: string
  seq: number
  in_turn: boolean
  queued: number
}

export interface Frame {
  stream: "event" | "gone"
  session: string
  event?: OmniEvent
  snapshot?: Snapshot
  /** Set on frames read back out of the log rather than seen live. */
  replay?: boolean
}

export type Ask =
  | { how: "key"; key: string }
  | { how: "intelligence"; value: number; bench?: string }
  | { how: "model"; model: string; provider?: string; effort?: string; fast?: boolean }

/** What should answer, in every shape this client accepts. */
export type Said =
  | string
  | number
  | Ask
  | { key: string }
  | { intelligence: number; bench?: string }
  | { model: string; provider?: string; effort?: string; fast?: boolean }

export interface Bridge {
  omni: "web"
  version: string
  protocol: number
  instance: string
  port: number
  started: string
}

export interface Grant {
  id: string
  name: string
  origin: string
  issued: string
  expires_in: number
}

export interface Opened {
  session: string
  replayed: number
  listeners: number
  snapshot: Snapshot
}

export interface LiveSession {
  snapshot: Snapshot
  listeners: number
}

export interface Pick {
  provider: string
  model: string
  effort: string
  fast: boolean
}

export declare class OmniError extends Error {
  status: number
  body: unknown
  constructor(message: string, options?: { status?: number; body?: unknown; cause?: unknown })
}

/** Nothing answered on any port the bridge uses. */
export declare class NoBridge extends OmniError {}

/** There is a bridge, but this site has no grant on it. Ask for a code. */
export declare class NotPaired extends OmniError {}

export declare function ports(): number[]
export declare function portFromCode(code: string): number | null
export declare function ask(said: Said): Ask
export declare function sse(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<{ id: string | null; event: string; data: string }>

export interface Options {
  port?: number
  omniauth?: string
  bridge?: Bridge | null
  store?: Storage | null
  fetch?: typeof fetch
}

export declare class Inference {
  port: number
  omniauth: string
  bridge: Bridge | null
  readonly url: string
  chats: Map<string, Chat>

  constructor(options: Options)

  /** Knock on every port the bridge uses; null when none answers. */
  static find(options?: { timeout?: number; fetch?: typeof fetch }): Promise<(Bridge & { port: number }) | null>
  /** Spend a code from `omni web connect`. Good once, for five minutes. */
  static pair(
    code: string,
    options?: { name?: string; port?: number | null; store?: Storage | null; fetch?: typeof fetch },
  ): Promise<Inference>
  /** Come back with the omniauth from last time, or throw `NotPaired`. */
  static connect(options?: Options): Promise<Inference>

  remember(): void
  forget(): void
  call(method: string, path: string, options?: { body?: unknown; signal?: AbortSignal; raw?: boolean }): Promise<any>

  hello(): Promise<Bridge>
  whoami(): Promise<{ grant: Grant; bridge: Bridge; attached: { session: string; readers: number }[] }>
  providers(only?: string[]): Promise<string[]>
  dial(providers?: string[]): Promise<Record<string, Pick>>
  sessions(): Promise<LiveSession[]>
  account(provider: string, what?: "status" | "installed" | "limits"): Promise<unknown>
  loadOrCreateSession(session: string, providers?: string[]): Chat
  disconnect(): Promise<{ disconnected: boolean }>
}

export declare class Chat {
  readonly session: string
  snapshot: Snapshot | null
  started: boolean
  /** True while the event stream is being read and reopened. */
  watching: boolean
  since: number

  model(said: Said): Promise<this>
  set(what: string, value: unknown): Promise<this>
  onEvent(handler: (event: OmniEvent) => void): () => void
  onError(handler: (error: Error) => void): () => void
  start(options?: { since?: number }): Promise<Opened>
  send(text: string): Promise<boolean>
  history(since?: number): Promise<OmniEvent[]>
  status(): Promise<{ snapshot: Snapshot; listeners: number }>
  detach(): Promise<void>
  stop(): Promise<boolean>
  close(): void
  events(options?: { until?: EventType | null }): AsyncGenerator<OmniEvent>
}

export default Inference
