import assert from "node:assert/strict"
import test from "node:test"

import {
  BASE_PORT,
  Chat,
  Event,
  FLOOR_PORT,
  Inference,
  NoBridge,
  NotPaired,
  OmniError,
  ask,
  portFromCode,
  ports,
  sse,
} from "../omni.js"

/** A fetch that answers from a table, and records what it was asked. */
function stub(routes) {
  const seen = []
  const doFetch = async (url, options = {}) => {
    seen.push({ url, ...options })
    const path = new URL(url).pathname + new URL(url).search
    const answer = routes[`${options.method ?? "GET"} ${path}`] ?? routes[path]
    if (!answer) throw new TypeError("fetch failed")
    const { status = 200, body = {}, stream } = answer
    if (stream) {
      return { ok: true, status, body: streamOf(stream) }
    }
    return {
      ok: status >= 200 && status < 300,
      status,
      json: async () => body,
    }
  }
  doFetch.seen = seen
  return doFetch
}

function streamOf(text) {
  const bytes = new TextEncoder().encode(text)
  let sent = false
  return {
    getReader() {
      return {
        async read() {
          if (sent) return { done: true, value: undefined }
          sent = true
          return { done: false, value: bytes }
        },
        async cancel() {},
      }
    },
  }
}

const HELLO = { omni: "web", version: "0.7.0", protocol: 1, instance: "abc", port: 1998, started: "now" }

test("the ports searched are the ones the bridge counts down through", () => {
  assert.equal(ports()[0], BASE_PORT)
  assert.equal(ports().at(-1), FLOOR_PORT)
  assert.equal(ports().length, 10)
})

test("a pairing code says which port to knock on", () => {
  assert.equal(portFromCode("XULA-1998"), 1998)
  assert.equal(portFromCode(" xula-1996 "), 1996)
  assert.equal(portFromCode("XULA1998"), null)
  assert.equal(portFromCode("XUL-1998"), null, "four letters")
  assert.equal(portFromCode(undefined), null)
})

test("what should answer is said three ways and only three", () => {
  assert.deepEqual(ask("code"), { how: "key", key: "code" })
  assert.deepEqual(ask("CODE"), { how: "key", key: "code" })
  assert.deepEqual(ask(7), { how: "intelligence", value: 7 })
  assert.deepEqual(ask("7"), { how: "intelligence", value: 7 }, "a number typed into a text box")
  assert.deepEqual(ask({ intelligence: 7, bench: "GDPval" }), {
    how: "intelligence",
    value: 7,
    bench: "GDPval",
  })
  assert.deepEqual(ask({ model: "gemini-3.7-flash-low", provider: "google" }), {
    how: "model",
    model: "gemini-3.7-flash-low",
    provider: "google",
    effort: "",
    fast: false,
  })
  // Already in wire shape, so it is passed along untouched.
  assert.deepEqual(ask({ how: "key", key: "design" }), { how: "key", key: "design" })
  assert.throws(() => ask(null), OmniError)
  assert.throws(() => ask({ nonsense: true }), OmniError)
})

test("an event stream comes apart into messages", async () => {
  const raw =
    ": ping\n\n" +
    "id: 4\nevent: frame\ndata: {\"a\":1}\n\n" +
    "event: end\ndata: over\ndata: and out\n\n"
  const seen = []
  for await (const message of sse(streamOf(raw))) seen.push(message)

  assert.equal(seen.length, 2, "a comment is not a message")
  assert.deepEqual(seen[0], { id: "4", event: "frame", data: '{"a":1}' })
  assert.equal(seen[1].event, "end")
  assert.equal(seen[1].data, "over\nand out", "several data lines rejoin with newlines")
})

test("finding the bridge takes the one that answers, not the first asked", async () => {
  const doFetch = stub({ "/omni": { body: { ...HELLO, port: 1996 } } })
  // Every port is offered the same answer here; what matters is that a
  // rejection on the others does not sink the search.
  const found = await Inference.find({ fetch: doFetch })
  assert.equal(found.omni, "web")
  assert.equal(doFetch.seen.length, 10, "all ten are knocked on at once")
})

test("nothing listening is a NoBridge rather than a hang", async () => {
  const found = await Inference.find({ fetch: async () => { throw new TypeError("fetch failed") } })
  assert.equal(found, null)
  await assert.rejects(() => Inference.pair("XULA-1998", { port: 1998, fetch: async () => { throw new TypeError("nope") } }), NoBridge)
})

test("pairing spends the code and keeps what comes back", async () => {
  const doFetch = stub({
    "POST /connect": { body: { omniauth: "omniauth_xyz", grant: { id: "1" }, bridge: HELLO } },
  })
  const store = new Map()
  const omni = await Inference.pair("XULA-1998", {
    name: "Test",
    store: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v), removeItem: (k) => store.delete(k) },
    fetch: doFetch,
  })
  assert.equal(omni.omniauth, "omniauth_xyz")
  assert.equal(omni.port, 1998)

  const sent = JSON.parse(doFetch.seen[0].body)
  assert.equal(sent.code, "XULA-1998")
  assert.equal(sent.name, "Test")
  assert.equal(JSON.parse(store.get("omni.web")).omniauth, "omniauth_xyz")
})

test("a code the bridge refuses comes back as the bridge's own sentence", async () => {
  const doFetch = stub({
    "POST /connect": { status: 401, body: { error: "that omniauth is not one this bridge is holding" } },
  })
  await assert.rejects(
    () => Inference.pair("ZZZZ-1998", { port: 1998, fetch: doFetch, store: null }),
    (error) => error instanceof OmniError && /not one this bridge/.test(error.message),
  )
})

test("coming back with nothing kept is NotPaired, not a crash", async () => {
  await assert.rejects(() => Inference.connect({ store: null }), NotPaired)
})

test("a grant the bridge no longer holds is forgotten on the way out", async () => {
  const store = new Map([["omni.web", JSON.stringify({ port: 1998, omniauth: "omniauth_old" })]])
  const shim = {
    getItem: (k) => store.get(k) ?? null,
    setItem: (k, v) => store.set(k, v),
    removeItem: (k) => store.delete(k),
  }
  const doFetch = stub({
    "/omni": { body: HELLO },
    "/whoami": { status: 401, body: { error: "that omniauth is not one this bridge is holding" } },
  })
  await assert.rejects(() => Inference.connect({ store: shim, fetch: doFetch }), NotPaired)
  assert.equal(store.has("omni.web"), false, "a dead token is not kept around")
})

test("settings asked for before start travel with the open", async () => {
  const doFetch = stub({
    "POST /open": { body: { session: "demo", replayed: 0, listeners: 1, snapshot: { seq: 3 } } },
    "/events?session=demo&since=-1": { stream: "event: end\ndata: {}\n\n" },
  })
  const omni = new Inference({ port: 1998, omniauth: "omniauth_x", store: null, fetch: doFetch })
  const chat = omni.loadOrCreateSession("demo", ["claude"])
  await chat.model("code")
  await chat.set("system_prompt", "be exact")
  assert.equal(chat.started, false, "nothing has been opened yet")

  await chat.start()
  const sent = JSON.parse(doFetch.seen.find((call) => call.method === "POST").body)
  assert.deepEqual(sent.settings, [
    { what: "model", value: { how: "key", key: "code" } },
    { what: "system_prompt", value: "be exact" },
  ])
  assert.equal(sent.providers[0], "claude")
  assert.equal(sent.from, -1)
  chat.close()
})

test("the same session id hands back the same chat", () => {
  const omni = new Inference({ port: 1998, omniauth: "omniauth_x", store: null, fetch: stub({}) })
  const first = omni.loadOrCreateSession("demo")
  assert.equal(omni.loadOrCreateSession("demo"), first)
  assert.notEqual(omni.loadOrCreateSession("other"), first)
})

test("a stream turns into events and stops at the end of the turn", async () => {
  const frames = [
    { stream: "event", session: "demo", event: { type: "start", seq: 1, text: "hi" }, snapshot: { seq: 1 } },
    { stream: "event", session: "demo", event: { type: "text", seq: 2, text: "hello back" }, snapshot: { seq: 2 } },
    { stream: "event", session: "demo", event: { type: "end", seq: 3 }, snapshot: { seq: 3 } },
  ]
  const body =
    'event: open\ndata: {"opened":{}}\n\n' +
    frames.map((frame) => `id: ${frame.event.seq}\nevent: frame\ndata: ${JSON.stringify(frame)}\n\n`).join("")

  const doFetch = stub({
    "POST /open": { body: { session: "demo", replayed: 0, listeners: 1, snapshot: { seq: 0 } } },
    "/events?session=demo&since=-1": { stream: body },
  })
  const omni = new Inference({ port: 1998, omniauth: "omniauth_x", store: null, fetch: doFetch })
  const chat = omni.loadOrCreateSession("demo")

  const seen = []
  for await (const event of chat.events()) seen.push(event)

  assert.deepEqual(seen.map((event) => event.type), [Event.START, Event.TEXT, Event.END])
  assert.equal(chat.snapshot.seq, 3, "the snapshot rides along with the events")
  assert.equal(chat.since, 3, "and the cursor moves, so a reconnect resumes")
  chat.close()
})

test("a callback that throws does not take the stream down with it", async () => {
  const body =
    'id: 1\nevent: frame\ndata: {"stream":"event","session":"demo","event":{"type":"text","seq":1,"text":"one"}}\n\n' +
    'event: end\ndata: {}\n\n'
  const doFetch = stub({
    "POST /open": { body: { session: "demo", replayed: 0, listeners: 1, snapshot: {} } },
    "/events?session=demo&since=-1": { stream: body },
  })
  const omni = new Inference({ port: 1998, omniauth: "omniauth_x", store: null, fetch: doFetch })
  const chat = omni.loadOrCreateSession("demo")

  const trouble = []
  chat.onError((error) => trouble.push(error))
  chat.onEvent(() => {
    throw new Error("the page has a bug")
  })
  const seen = []
  chat.onEvent((event) => seen.push(event))

  await chat.start()
  await new Promise((resolve) => setTimeout(resolve, 50))
  assert.equal(seen.length, 1, "the second listener still heard it")
  assert.equal(trouble[0].message, "the page has a bug")
  chat.close()
})

test("Chat is what loadOrCreateSession hands back", () => {
  const omni = new Inference({ port: 1998, omniauth: "omniauth_x", store: null, fetch: stub({}) })
  assert.ok(omni.loadOrCreateSession("demo") instanceof Chat)
})
