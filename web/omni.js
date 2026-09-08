/**
 * omni, from a browser.
 *
 * The same vocabulary as the Python and Rust clients — an `Inference` opens a
 * `Chat`, everything it emits is an `Event` — over the loopback bridge that
 * `omni web` puts in front of the daemon. Nothing is bundled and nothing is
 * imported: one file, standard `fetch`, works in a browser and in Node.
 *
 * The one thing this has that the others do not is pairing. A Unix socket is
 * already proof you are the person at the keyboard; a port on localhost is
 * not, because every page you have open can reach it too. So somebody runs
 * `omni web connect`, reads out four letters, and the site trades them once
 * for an omniauth it keeps.
 *
 *     import { Inference } from "@teamofsilicons/omni-web"
 *
 *     const omni = await Inference.connect()
 *         .catch(() => Inference.pair(prompt("code from `omni web connect`?")))
 *
 *     const chat = omni.loadOrCreateSession("my-session")
 *     await chat.model("code")
 *     chat.onEvent(event => {
 *       if (event.type === Event.TEXT) console.log(event.text)
 *     })
 *     await chat.start()
 *     await chat.send("what changed in this repo today?")
 */

/** Where the bridge would rather be. */
export const BASE_PORT = 1998
/** How far down it counts before giving up, and so how far down we look. */
export const FLOOR_PORT = 1989
/** Where a resumed omniauth is kept, when there is somewhere to keep it. */
export const STORAGE_KEY = "omni.web"

/**
 * One thing that happened. The class holds the types; an instance is a plain
 * object off the wire, exactly as Python and Rust see it.
 */
export const Event = Object.freeze({
  START: "start",
  TEXT: "text",
  THINKING: "thinking",
  TOOL: Object.freeze({ CALL: "tool.call", RESULT: "tool.result" }),
  TOOL_CALL: "tool.call",
  TOOL_RESULT: "tool.result",
  END: "end",
  INJECTED: "injected",
  ERROR: "error",
  SWITCH_PROVIDER: "switch_provider",
  NEW_SESSION: "new_session",
  CONFIG: "config",
})

/** The four `event.kind` values that classify an ERROR from a provider. */
export const Kind = Object.freeze({
  AUTH: "auth",
  LIMIT: "limit",
  UNAVAILABLE: "unavailable",
  CRASH: "crash",
})

export class OmniError extends Error {
  constructor(message, { status = 0, body = null, cause = undefined } = {}) {
    super(message, { cause })
    this.name = "OmniError"
    this.status = status
    this.body = body
  }
}

/** Nothing answered on any of the ports the bridge uses. */
export class NoBridge extends OmniError {
  constructor(message = "no omni bridge is listening; run `omni web` on this machine", options) {
    super(message, options)
    this.name = "NoBridge"
  }
}

/**
 * There is a bridge, but this site is not allowed through it — never paired,
 * or paired to a bridge that has since moved. Both are fixed the same way, so
 * they are the same error: ask for a code.
 */
export class NotPaired extends OmniError {
  constructor(
    message = "this site is not paired; run `omni web connect` and pass the code to Inference.pair()",
    options,
  ) {
    super(message, options)
    this.name = "NotPaired"
  }
}

/** Every port the bridge could be on, nearest preference first. */
export function ports() {
  const all = []
  for (let port = BASE_PORT; port >= FLOOR_PORT; port--) all.push(port)
  return all
}

/**
 * The port hidden in a pairing code.
 *
 * `XULA-1998` says where to knock as well as what to say, which is the whole
 * reason the port is in there — a site given a code never has to search.
 */
export function portFromCode(code) {
  const match = /^\s*([A-Za-z]{4})-(\d{2,5})\s*$/.exec(String(code ?? ""))
  return match ? Number(match[2]) : null
}

function origin(port) {
  return `http://127.0.0.1:${port}`
}

function storage(given) {
  if (given !== undefined) return given
  try {
    return globalThis.localStorage ?? null
  } catch {
    // A page with storage blocked should still work; it just re-pairs.
    return null
  }
}

async function withTimeout(run, ms) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), ms)
  try {
    return await run(controller.signal)
  } finally {
    clearTimeout(timer)
  }
}

/**
 * The front door. Holds the bridge it found and the omniauth it holds, and
 * hands out a `Chat` per session.
 */
export class Inference {
  /**
   * Ask each port the bridge might be on whether it is one.
   *
   * All of them at once: ten loopback requests that fail instantly cost less
   * than one that has to time out, and only one of them can answer.
   */
  static async find({ timeout = 1500, fetch: doFetch = globalThis.fetch } = {}) {
    const knocks = ports().map((port) =>
      withTimeout(
        (signal) =>
          doFetch(`${origin(port)}/omni`, { signal, headers: { accept: "application/json" } })
            .then((response) => (response.ok ? response.json() : Promise.reject(new Error("not ok"))))
            .then((hello) => (hello?.omni === "web" ? { ...hello, port } : Promise.reject(new Error("not omni")))),
        timeout,
      ),
    )
    // `Promise.any` gives the first that answers rather than the first that
    // was asked, which on a cold machine is a different port.
    try {
      return await Promise.any(knocks)
    } catch {
      return null
    }
  }

  /**
   * Spend a pairing code and keep what it gives back.
   *
   * The code is good once and for five minutes. Everything after this is the
   * omniauth, which the browser keeps until it idles out.
   */
  static async pair(code, { name = documentName(), port = null, store, fetch: doFetch = globalThis.fetch } = {}) {
    const where = port ?? portFromCode(code)
    const bridge = where ? { port: where } : await Inference.find({ fetch: doFetch })
    if (!bridge) throw new NoBridge()

    let response
    try {
      response = await doFetch(`${origin(bridge.port)}/connect`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ code: String(code ?? "").trim(), name }),
      })
    } catch (cause) {
      throw new NoBridge(`nothing answered at ${origin(bridge.port)}`, { cause })
    }
    const answer = await response.json().catch(() => null)
    if (!response.ok) {
      throw new OmniError(answer?.error ?? `pairing failed (${response.status})`, {
        status: response.status,
        body: answer,
      })
    }
    const inference = new Inference({
      port: answer.bridge.port,
      omniauth: answer.omniauth,
      bridge: answer.bridge,
      store,
      fetch: doFetch,
    })
    inference.remember()
    return inference
  }

  /**
   * Come back with the omniauth from last time.
   *
   * Throws `NotPaired` when there is nothing kept or what was kept is no
   * longer good — which is one case, not two, because a site does the same
   * thing about both: ask for a code.
   */
  static async connect({ store, omniauth, port, fetch: doFetch = globalThis.fetch } = {}) {
    const kept = omniauth ? { omniauth, port } : read(storage(store))
    if (!kept?.omniauth) throw new NotPaired()

    // The port it was paired on is the only one worth trying: a grant does not
    // survive the bridge moving, so finding one elsewhere would not help.
    const bridge = await withTimeout(
      (signal) =>
        doFetch(`${origin(kept.port ?? BASE_PORT)}/omni`, { signal })
          .then((response) => (response.ok ? response.json() : null))
          .catch(() => null),
      1500,
    )
    if (!bridge) throw new NoBridge()

    const inference = new Inference({
      port: kept.port ?? BASE_PORT,
      omniauth: kept.omniauth,
      bridge,
      store,
      fetch: doFetch,
    })
    // Prove the grant before handing back something that looks connected.
    await inference.whoami()
    inference.remember()
    return inference
  }

  constructor({ port = BASE_PORT, omniauth, bridge = null, store, fetch: doFetch = globalThis.fetch } = {}) {
    if (!omniauth) throw new NotPaired("an Inference needs an omniauth")
    this.port = port
    this.omniauth = omniauth
    this.bridge = bridge
    this.store = storage(store)
    this.fetch = doFetch
    this.chats = new Map()
  }

  get url() {
    return origin(this.port)
  }

  /** Keep the omniauth for next time, when there is somewhere to keep it. */
  remember() {
    try {
      this.store?.setItem(STORAGE_KEY, JSON.stringify({ port: this.port, omniauth: this.omniauth }))
    } catch {
      /* a page with storage blocked pairs again next time, which is fine */
    }
  }

  forget() {
    try {
      this.store?.removeItem(STORAGE_KEY)
    } catch {
      /* nothing to do about it */
    }
  }

  async call(method, path, { body, signal, raw = false } = {}) {
    const headers = { authorization: `Bearer ${this.omniauth}` }
    if (body !== undefined) headers["content-type"] = "application/json"
    let response
    try {
      response = await this.fetch(`${this.url}${path}`, {
        method,
        headers,
        signal,
        body: body === undefined ? undefined : JSON.stringify(body),
      })
    } catch (cause) {
      throw new NoBridge(`the bridge at ${this.url} stopped answering`, { cause })
    }
    // A streaming caller wants the body, not a parsed object. Everything
    // below is how a failure is reported, and that is JSON either way.
    if (raw && response.ok) return response
    const answer = await response.json().catch(() => null)
    if (response.ok) return answer
    if (response.status === 401 || response.status === 403) {
      this.forget()
      throw new NotPaired(answer?.error ?? "this site is no longer paired")
    }
    throw new OmniError(answer?.error ?? `omni answered ${response.status}`, {
      status: response.status,
      body: answer,
    })
  }

  hello() {
    return this.call("GET", "/omni")
  }

  whoami() {
    return this.call("GET", "/whoami")
  }

  /** Which providers are installed *and* signed in. Nothing else is offered. */
  async providers(only) {
    const query = only?.length ? `?only=${encodeURIComponent(only.join(","))}` : ""
    return this.call("GET", `/providers${query}`)
  }

  /** What each rung of the 0–10 dial resolves to right now. */
  async dial(providers) {
    const query = providers?.length ? `?providers=${encodeURIComponent(providers.join(","))}` : ""
    return this.call("GET", `/dial${query}`)
  }

  async sessions() {
    const answer = await this.call("GET", "/sessions")
    return answer.sessions ?? []
  }

  /**
   * Read a provider account: `status`, `installed`, or `limits`.
   *
   * Only reads. Signing a provider in happens at a terminal, and the bridge
   * refuses to start it however good this token is.
   */
  async account(provider, what = "status") {
    return this.call("GET", `/account?provider=${encodeURIComponent(provider)}&what=${encodeURIComponent(what)}`)
  }

  /** The durable conversation with this id, creating it if it is new. */
  loadOrCreateSession(session, providers) {
    const held = this.chats.get(session)
    if (held) return held
    const chat = new Chat(this, session, providers)
    this.chats.set(session, chat)
    return chat
  }

  /** Give the omniauth back. Everything this site had open is let go. */
  async disconnect() {
    for (const chat of this.chats.values()) chat.close()
    this.chats.clear()
    try {
      return await this.call("POST", "/disconnect", { body: {} })
    } finally {
      this.forget()
    }
  }
}

/**
 * What should answer, said the way the rest of omni says it.
 *
 *   "code"                       a shortlist somebody chose
 *   7                            the dial
 *   {intelligence: 7, bench}     the dial, on a particular board
 *   {model, provider, effort, fast}   you already know
 */
export function ask(said) {
  if (said === null || said === undefined) throw new OmniError("model needs a word, a number, or a model")
  if (typeof said === "number") return { how: "intelligence", value: said }
  if (typeof said === "string") {
    const number = Number(said)
    if (said.trim() !== "" && Number.isInteger(number)) return { how: "intelligence", value: number }
    return { how: "key", key: said.trim().toLowerCase() }
  }
  if (said.how) return said
  if (said.key !== undefined) return { how: "key", key: String(said.key).trim().toLowerCase() }
  if (said.intelligence !== undefined) {
    const dial = { how: "intelligence", value: Number(said.intelligence) }
    if (said.bench) dial.bench = said.bench
    return dial
  }
  if (said.model !== undefined) {
    const named = { how: "model", model: String(said.model), effort: said.effort ?? "", fast: !!said.fast }
    if (said.provider) named.provider = said.provider
    return named
  }
  throw new OmniError("model takes a key, an intelligence, or a model")
}

/**
 * One conversation, which outlives the page that opened it.
 *
 * Settings asked for before `start` travel with the open, so the first
 * provider comes up already configured rather than being started and then
 * immediately replaced.
 */
export class Chat {
  constructor(inference, session, providers) {
    this.inference = inference
    this.session = session
    this.providers = providers ?? null
    this.pending = []
    this.listeners = new Set()
    this.problems = new Set()
    this.snapshot = null
    this.started = false
    this.reading = null
    this.controller = null
    this.since = -1
    this.watching = false
    this.closed = false
  }

  /** What should answer. Queued before `start`, sent as a change after it. */
  async model(said) {
    const value = ask(said)
    if (!this.started) {
      this.pending.push({ what: "model", value })
      return this
    }
    await this.set("model", value)
    return this
  }

  /** Any other session setting: `system_prompt`, `mcp`, `subagents`, `cwd`… */
  async set(what, value) {
    if (!this.started) {
      this.pending.push({ what, value })
      return this
    }
    await this.inference.call("POST", "/set", { body: { session: this.session, what, value } })
    return this
  }

  /** Every event, as it happens. Returns the way to stop listening. */
  onEvent(handler) {
    this.listeners.add(handler)
    return () => this.listeners.delete(handler)
  }

  /** Stream trouble — the bridge going away, a reconnect — not model errors. */
  onError(handler) {
    this.problems.add(handler)
    return () => this.problems.delete(handler)
  }

  /**
   * Open the session and begin reading it.
   *
   * `since` is where replay starts: `-1` for whatever happens next, `0` for
   * the whole conversation so far, or any sequence number in between.
   */
  async start({ since = -1 } = {}) {
    if (this.closed) throw new OmniError(`chat ${this.session} was stopped`)
    const opened = await this.inference.call("POST", "/open", {
      body: {
        session: this.session,
        providers: this.providers ?? undefined,
        settings: this.pending,
        from: since,
      },
    })
    this.pending = []
    this.started = true
    this.snapshot = opened.snapshot
    this.since = since
    if (!this.reading) {
      this.watching = true
      this.reading = this.#read(since)
    }
    return opened
  }

  async send(text) {
    if (!this.started) await this.start()
    const answer = await this.inference.call("POST", "/send", {
      body: { session: this.session, text },
    })
    return !!answer.accepted
  }

  /** The persisted log, read once, without holding anything open. */
  async history(since = 0) {
    const answer = await this.inference.call(
      "GET",
      `/history?session=${encodeURIComponent(this.session)}&since=${since}`,
    )
    return answer.events ?? []
  }

  async status() {
    const answer = await this.inference.call("GET", `/status?session=${encodeURIComponent(this.session)}`)
    this.snapshot = answer.snapshot
    return answer
  }

  /** Stop listening. The daemon keeps the provider warm for the next caller. */
  async detach() {
    this.close()
    if (!this.started) return
    this.started = false
    await this.inference.call("POST", "/detach", { body: { session: this.session } })
  }

  /** End the conversation. The provider is shut down. */
  async stop() {
    this.close()
    this.started = false
    this.closed = true
    this.inference.chats.delete(this.session)
    const answer = await this.inference.call("POST", "/stop", { body: { session: this.session } })
    return !!answer.stopped
  }

  /** Drop the local stream without telling the bridge anything. */
  close() {
    // Before the abort, so a read that is between reconnects sees it too.
    this.watching = false
    this.controller?.abort()
    this.controller = null
    this.reading = null
  }

  /**
   * Events as an async iterator, for code that would rather await than
   * register a callback.
   *
   *     for await (const event of chat.events()) { … }
   *
   * It stops at the end of the turn. Pass `{until: null}` to keep going for
   * as long as the session does.
   */
  async *events({ until = Event.END } = {}) {
    const queue = []
    let wake = null
    const off = this.onEvent((event) => {
      queue.push(event)
      wake?.()
    })
    try {
      if (!this.started) await this.start()
      for (;;) {
        while (queue.length) {
          const event = queue.shift()
          yield event
          if (until && event.type === until) return
        }
        await new Promise((resolve) => {
          // Something may have arrived while the last one was being yielded.
          if (queue.length) return resolve()
          wake = resolve
        })
        wake = null
      }
    } finally {
      off()
    }
  }

  #emit(event) {
    for (const listener of this.listeners) {
      try {
        listener(event)
      } catch (error) {
        this.#trouble(error)
      }
    }
  }

  #trouble(error) {
    for (const handler of this.problems) {
      try {
        handler(error)
      } catch {
        /* a handler that throws about a throw is on its own */
      }
    }
  }

  /**
   * Read the event stream, and put it back when it breaks.
   *
   * A laptop that slept, a bridge that restarted, a network stack that
   * decided: all of them look the same from here, and all of them are fixed
   * by asking again from the last sequence actually seen. Nothing is lost,
   * because the daemon has the log and `since` is how you ask it for the rest.
   */
  async #read(since) {
    let from = since
    let backoff = 250
    while (this.watching && !this.closed) {
      this.controller = new AbortController()
      try {
        const response = await this.inference.call(
          "GET",
          `/events?session=${encodeURIComponent(this.session)}&since=${from}`,
          { signal: this.controller.signal, raw: true },
        )
        backoff = 250
        for await (const message of sse(response.body)) {
          if (message.event === "frame") {
            const frame = JSON.parse(message.data)
            if (frame.snapshot) this.snapshot = frame.snapshot
            if (frame.event) {
              from = frame.event.seq
              this.since = from
              this.#emit(frame.event)
            }
          } else if (message.event === "lag") {
            // The bridge says this reader fell behind and the stream has a
            // hole. The log does not, so the gap is filled from it.
            const missed = JSON.parse(message.data)
            for (const event of await this.history(from + 1)) {
              from = event.seq
              this.#emit(event)
            }
            this.#trouble(new OmniError(`caught up after missing ${missed.missed} frames`))
          } else if (message.event === "end") {
            return
          }
        }
      } catch (error) {
        if (!this.watching || this.closed || error?.name === "AbortError") return
        // A grant that is gone will not come back by trying again.
        if (error instanceof NotPaired) {
          this.#trouble(error)
          return
        }
        this.#trouble(error)
      }
      if (!this.watching || this.closed) return
      await new Promise((resolve) => setTimeout(resolve, backoff))
      backoff = Math.min(backoff * 2, 10_000)
    }
  }
}

/**
 * Server-sent events out of a `fetch` body.
 *
 * `EventSource` would do this, but it cannot set a header, so the omniauth
 * would have to go in the URL — where it lands in history, in referrers and
 * in logs. Forty lines of parsing is the cheaper side of that trade.
 */
export async function* sse(body) {
  const reader = body.getReader()
  const decode = new TextDecoder()
  let buffer = ""
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decode.decode(value, { stream: true })
      let split
      while ((split = buffer.indexOf("\n\n")) !== -1) {
        const block = buffer.slice(0, split)
        buffer = buffer.slice(split + 2)
        const message = { id: null, event: "message", data: "" }
        const lines = []
        for (const line of block.split("\n")) {
          if (line.startsWith(":")) continue
          const at = line.indexOf(":")
          const field = at === -1 ? line : line.slice(0, at)
          const value = at === -1 ? "" : line.slice(at + 1).replace(/^ /, "")
          if (field === "id") message.id = value
          else if (field === "event") message.event = value
          else if (field === "data") lines.push(value)
        }
        if (!lines.length && message.event === "message") continue
        message.data = lines.join("\n")
        yield message
      }
    }
  } finally {
    reader.cancel().catch(() => {})
  }
}

function documentName() {
  try {
    return globalThis.document?.title || globalThis.location?.host || ""
  } catch {
    return ""
  }
}

function read(store) {
  try {
    const raw = store?.getItem(STORAGE_KEY)
    return raw ? JSON.parse(raw) : null
  } catch {
    return null
  }
}

export default Inference
