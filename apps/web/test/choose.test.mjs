/**
 * The answer is worked out here and nowhere else, so it gets tested here.
 *
 * Run with `npm test`. No test framework: node's own runner, and the
 * typescript already installed for the build.
 */

import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { test } from "node:test"
import { pathToFileURL } from "node:url"

const require = createRequire(import.meta.url)
const ts = require("typescript")

function dataUrl(name) {
  const source = readFileSync(new URL(`../lib/${name}.ts`, import.meta.url), "utf8")
  return (
    "data:text/javascript," +
    encodeURIComponent(
      ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
      }).outputText.replace(/from "\.\/([a-z]+)"/g, (_, n) => `from "${dataUrl(n)}"`),
    )
  )
}

async function load(file) {
  const js = ts.transpileModule(readFileSync(new URL(`../lib/${file}`, import.meta.url), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  }).outputText
  return import(
    "data:text/javascript," +
      encodeURIComponent(js.replace(/from "\.\/([a-z]+)"/g, (_, n) => `from "${dataUrl(n)}"`))
  )
}

const { byKey, byIntelligence, byCuratedIntelligence, byName, edge, plottable, unusable, ladderOf } = await load("choose.ts")
const { invocation, canRunFast } = await load("providers.ts")

const ALL = ["claude-code-cli", "codex-app-server", "antigravity-cli"]
const DOC = JSON.parse(readFileSync(new URL("../models.json", import.meta.url), "utf8"))

// Exercise the actual route with the checked-in catalog, without a database.
const imports = {
  "next/server": pathToFileURL(require.resolve("next/server")).href,
  "../../lib/choose": dataUrl("choose"),
  "../../lib/providers": dataUrl("providers"),
  "../../lib/graph": `data:text/javascript,${encodeURIComponent(`export const VIEW = "test-catalog"; export async function graph() { return ${JSON.stringify({ ...DOC, fresh: false })}; }`)}`,
}
const routeSource = ts.transpileModule(readFileSync(new URL("../app/choose.json/route.ts", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText.replace(/from "([^"]+)"/g, (_, name) => `from "${imports[name] ?? name}"`)
const route = await import(`data:text/javascript,${encodeURIComponent(routeSource)}`)
const askRoute = query => route.GET({ nextUrl: new URL(`https://backend.omni.teamofsilicons.com/choose.json?${query}`) })

/* ------------------------------------------------------------------ by key */

test("a key answers with the first vendor on its list you are signed into", () => {
  const keys = {
    code: {
      says: "write code",
      picks: [
        { provider: "codex-app-server", model: "gpt-6-astra", effort: "max" },
        { provider: "claude-code-cli", model: "opus", effort: "max" },
      ],
    },
  }
  assert.equal(byKey(keys, "code", ALL).pick.provider, "codex-app-server")
  assert.equal(byKey(keys, "code", ["claude-code-cli", "antigravity-cli"]).pick.provider, "claude-code-cli")
})

test("losing every vendor on a list is an honest nothing, not a substitute", () => {
  const keys = { code: { says: "…", picks: [{ provider: "codex-app-server", model: "gpt-6-astra", effort: "max" }] } }
  assert.equal(byKey(keys, "code", ["antigravity-cli"]).pick, null)
})

test("the whole shortlist comes back, marked with what was available", () => {
  const keys = {
    code: {
      says: "…",
      picks: [
        { provider: "codex-app-server", model: "gpt-6-astra", effort: "max" },
        { provider: "claude-code-cli", model: "opus", effort: "max" },
      ],
    },
  }
  const a = byKey(keys, "code", ["claude-code-cli"])
  assert.deepEqual(a.shortlist.map((p) => p.available), [false, true])
})

test("an unknown key is refused rather than guessed at", () => {
  for (const key of ["nonsense", "constructor", "__proto__", "toString", "hasOwnProperty"]) {
    assert.equal(byKey(DOC.keys, key, ALL), null, key)
  }
})

test("Argon requires a positive exact model match and keeps the Flash fallback in provider order", () => {
  const providers = ["antigravity-cli", "codex-app-server"]
  const available = { "antigravity-cli": ["gemini-4-argon-high"] }
  assert.equal(byKey(DOC.keys, "design", providers).pick.model, "gemini-3.8-flash-high")
  assert.equal(byKey(DOC.keys, "design", providers, available).pick.model, "gemini-4-argon-high")
  assert.equal(byKey(DOC.keys, "design", ["codex-app-server"], available).pick.model, "gpt-6-astra")
  for (const models of [{}, { "antigravity-cli": [] }, { "antigravity-cli": ["gemini-4-argon-medium"] }, { "codex-app-server": ["gemini-4-argon-high"] }]) {
    const answer = byKey(DOC.keys, "design", providers, models)
    assert.equal(answer.pick.model, "gemini-3.8-flash-high")
    assert.equal(answer.shortlist.find(p => p.model === "gemini-4-argon-high").available, false)
  }
  assert.equal(byKey(DOC.keys, "design", ALL, available).pick.model, "claude-opus-5-5")
})

test("all seven configured category defaults match the requested efforts and priorities", () => {
  const expected = {
    fast: ["gpt-6.1-sol", "high", true], cost: ["gpt-6.1-sol", "low", false],
    code: ["claude-opus-5-5", "max", false], general: ["claude-sonnet-5-5", "max", false],
    design: ["claude-opus-5-5", "max", false], research: ["gpt-6-astra", "max", false],
    writing: ["claude-sonnet-5-5", "max", false],
  }
  assert.deepEqual(Object.keys(DOC.keys).sort(), Object.keys(expected).sort())
  for (const [key, pick] of Object.entries(expected)) {
    const actual = byKey(DOC.keys, key, ALL).pick
    assert.deepEqual([actual.model, actual.effort, actual.fast], pick, key)
    const gemini = byKey(DOC.keys, key, ["antigravity-cli"]).pick
    assert.equal(gemini.model, "gemini-3.8-flash-high", key)
  }
})

test("the route validates and applies availability to keys and the curated dial", async () => {
  for (const field of ["key=writing", "intelligence=7"]) {
    const query = `${field}&providers=antigravity-cli`
    assert.equal((await (await askRoute(query)).json()).pick.model, "gemini-3.8-flash-high")
    const evidence = encodeURIComponent(JSON.stringify({ "antigravity-cli": ["gemini-4-argon-high"] }))
    const result = await (await askRoute(`${query}&available_models=${evidence}`)).json()
    assert.equal(result.pick.model, "gemini-4-argon-high")
  }
  for (const bad of ["invalid-json", "null", "[]", '{"antigravity-cli":"gemini-4-argon-high"}', '{"unknown":[]}', '{"antigravity-cli":[null]}']) {
    assert.equal((await askRoute(`key=code&available_models=${encodeURIComponent(bad)}`)).status, 400)
  }
})

/* ------------------------------------------------------------------- fast */

test("fast survives on a CLI that has one, and is dropped on one that does not", () => {
  const keys = {
    fast: {
      says: "…",
      picks: [
        { provider: "antigravity-cli", model: "gem", effort: "", fast: true },
        { provider: "codex-app-server", model: "gpt-6-astra", effort: "max", fast: true },
      ],
    },
  }
  const list = byKey(keys, "fast", ALL).shortlist
  assert.equal(list[0].fast, false, "Antigravity has no fast mode, so asking is ignored")
  assert.equal(list[1].fast, true, "Codex does")
})

test("fast changes the command, and only where it means something", () => {
  assert.match(invocation({ provider: "codex-app-server", model: "gpt-6-astra", effort: "max", fast: true }), /service_tier="fast"/)
  assert.match(invocation({ provider: "claude-code-cli", model: "opus", effort: "low", fast: true }), /\/fast/)
  assert.equal(
    invocation({ provider: "antigravity-cli", model: "gem", effort: "", fast: true }),
    invocation({ provider: "antigravity-cli", model: "gem", effort: "" }),
  )
})

test("Sonnet never advertises Claude fast mode or silently changes to Opus", () => {
  assert.equal(canRunFast("claude-code-cli", "claude-sonnet-5-5"), false)
  assert.equal(canRunFast("claude-code-cli", "claude-opus-5-5"), true)
  assert.doesNotMatch(invocation({ provider: "claude-code-cli", model: "claude-sonnet-5-5", effort: "medium", fast: true }), /\/fast/)
  const answer = byName(ALL, { provider: "claude-code-cli", model: "claude-sonnet-5-5", effort: "medium", fast: true })
  assert.equal(answer.pick, null)
  assert.match(answer.why, /does not support/)
})

test("only two of the three can run hot", () => {
  assert.deepEqual(ALL.map(id => canRunFast(id)), [true, true, false])
})

/* --------------------------------------------------------------- by number */

const BOARD = [
  { provider: "claude-code-cli", model: "top", effort: "max", scores: { b: 1800 }, price: 30, secs: 400 },
  { provider: "codex-app-server", model: "good", effort: "high", scores: { b: 1600 }, price: 8, secs: 100 },
  { provider: "claude-code-cli", model: "dear", effort: "high", scores: { b: 1500 }, price: 12, secs: 90 },
  { provider: "codex-app-server", model: "mid", effort: "medium", scores: { b: 1300 }, price: 3, secs: 40 },
  { provider: "codex-app-server", model: "floor", effort: "low", scores: { b: 900 }, price: 0.5, secs: 20 },
]

test("a model nothing beats on both counts stays on the edge", () => {
  assert.deepEqual(edge(plottable(BOARD, "b"), "b").map((p) => p.model), ["top", "good", "mid", "floor"])
})

test("a model that is worse and dearer is never offered", () => {
  assert.ok(!edge(plottable(BOARD, "b"), "b").some((p) => p.model === "dear"))
})

test("the edge runs down and to the left", () => {
  const walk = edge(plottable(BOARD, "b"), "b")
  for (let i = 0; i < walk.length - 1; i++) {
    assert.ok(walk[i].scores.b > walk[i + 1].scores.b, "score must fall")
    assert.ok(walk[i].price > walk[i + 1].price, "price must fall")
  }
})

test("ten is the top of the edge and nought is the bottom", () => {
  const ladder = ladderOf(edge(plottable(BOARD, "b"), "b"))
  assert.equal(ladder["10"].model, "top")
  assert.equal(ladder["0"].model, "floor")
})

test("the dial never goes up as the number goes down", () => {
  const a = byIntelligence(BOARD, ALL, "b", 9)
  const b = byIntelligence(BOARD, ALL, "b", 2)
  assert.notEqual(a.pick.model, b.pick.model)
  assert.equal(byIntelligence(BOARD, ALL, "b", 99).intelligence, 10, "clamped, not thrown")
  assert.equal(byIntelligence(BOARD, ALL, "b", -4).intelligence, 0)
})

test("losing a provider puts shadowed models back on the dial", () => {
  const all = byIntelligence(BOARD, ALL, "b", 10)
  const without = byIntelligence(BOARD, ["codex-app-server"], "b", 10)
  assert.equal(all.pick.model, "top")
  assert.equal(without.pick.model, "good")
})

test("automatic measured picks exclude legacy and restricted rows without hiding graph points", () => {
  const entries = [
    { ...BOARD[0], model: "old-winner", status: "legacy" },
    { ...BOARD[0], model: "restricted-winner", status: "restricted" },
    { ...BOARD[1], status: "current" },
  ]
  assert.equal(plottable(entries, "b").length, 3)
  const answer = byIntelligence(entries, ALL, "b", 10)
  assert.equal(answer.pick.model, "good")
  assert.equal(answer.basis, "measured")
  assert.equal(unusable(entries, "b").length, 2)
  const none = byIntelligence(entries.slice(0, 2), ALL, "b", 10)
  assert.equal(none.pick, null)
  assert.deepEqual(none.ladder, {})
})

test("the default dial uses the documented categories and current models without scores", () => {
  const categories = ["cost", "cost", "cost", "fast", "general", "general", "general", "code", "code", "research", "research"]
  for (const [level, category] of categories.entries()) {
    const answer = byCuratedIntelligence(DOC.keys, ALL, level)
    assert.equal(answer.basis, "curated")
    assert.equal(answer.category, category)
    assert.equal(answer.bench, undefined)
    assert.deepEqual(answer.pick, byKey(DOC.keys, category, ALL).pick)
    assert.equal(Object.keys(answer.ladder).length, 11)
    for (const pick of Object.values(answer.ladder)) {
      assert.ok(pick)
      assert.ok(!pick.model.includes("5.6"), `legacy default: ${pick.model}`)
      const entry = DOC.models.find((row) => row.provider === pick.provider && row.model === pick.model && (row.effort ?? "") === pick.effort)
      assert.equal(entry.status, "current")
    }
  }
})

test("an unavailable curated category keeps its rung null instead of inventing a substitute", () => {
  const answer = byCuratedIntelligence({ code: { says: "code", picks: [BOARD[1]] } }, ["claude-code-cli"], 7)
  assert.equal(answer.pick, null)
  assert.equal(Object.keys(answer.ladder).length, 11)
  assert.ok(Object.values(answer.ladder).every((pick) => pick === null))
})

test("adding measurements cannot bypass a model availability requirement", () => {
  const gated = { ...BOARD[0], requires_availability: true }
  assert.equal(byIntelligence([gated], ALL, "b", 10).pick, null)
  const models = { [gated.provider]: [gated.model] }
  assert.equal(byIntelligence([gated], ALL, "b", 10, models).pick.model, gated.model)
})

test("the public route defaults to curated picks and keeps benchmark queries measured", async () => {
  const defaultResponse = await askRoute("intelligence=7")
  assert.equal(defaultResponse.status, 200)
  const curated = await defaultResponse.json()
  assert.equal(curated.basis, "curated")
  assert.equal(curated.category, "code")
  assert.equal(curated.bench, undefined)
  assert.equal(curated.benchmark, undefined)
  assert.equal(Object.keys(curated.ladder).length, 11)
  assert.equal(curated.pick.model, byKey(DOC.keys, "code", ALL).pick.model)

  const measuredResponse = await askRoute("intelligence=7&bench=gdpval-aa")
  const measured = await measuredResponse.json()
  assert.equal(measured.basis, "measured")
  assert.equal(measured.bench, "gdpval-aa")
  assert.ok(measured.benchmark.label)
  assert.ok(Object.values(measured.ladder).every((pick) => !pick.model.includes("5.6")))
  assert.equal((await askRoute("intelligence=7&bench=unknown")).status, 400)
  assert.equal((await askRoute("intelligence=11")).status, 400)
  assert.equal((await askRoute("key=constructor")).status, 400)
  assert.equal((await askRoute("key=__proto__")).status, 400)
})

test("a measured endpoint with no eligible model returns null and a reason", async () => {
  const empty = Object.keys(DOC.benches).find((bench) => !byIntelligence(DOC.models, ALL, bench, 7).pick)
  assert.ok(empty, "the current catalog has unmeasured design/research boards")
  const response = await askRoute(`intelligence=7&bench=${encodeURIComponent(empty)}`)
  const answer = await response.json()
  assert.equal(response.status, 200)
  assert.equal(answer.pick, null)
  assert.deepEqual(answer.ladder, {})
  assert.match(answer.reason, /no current model/)
})

/* ----------------------------------------------------------------- by name */

test("a model you name is passed through, not second-guessed", () => {
  const a = byName(ALL, { provider: "codex-app-server", model: "gpt-released-this-morning", effort: "high" })
  assert.equal(a.pick.model, "gpt-released-this-morning")
  assert.match(invocation(a.pick), /gpt-released-this-morning/)
})

test("an effort the CLI does not take is refused with a reason", () => {
  const a = byName(ALL, { provider: "antigravity-cli", model: "gem", effort: "max" })
  assert.equal(a.pick, null)
  assert.match(a.why, /does not take the effort/)
})

test("a provider you are not signed into is refused rather than substituted", () => {
  const a = byName(["codex-app-server"], { provider: "claude-code-cli", model: "opus", effort: "max" })
  assert.equal(a.pick, null)
  assert.match(a.why, /not signed in/)
})

/* ------------------------------------------------------------- the real file */

test("every curated pick is a model the board actually carries", () => {
  const have = new Set(DOC.models.map((m) => `${m.provider}/${m.model}/${m.effort}`))
  for (const [id, k] of Object.entries(DOC.keys)) {
    assert.ok(k.says, `${id} must say what it is for`)
    for (const p of k.picks) {
      assert.ok(have.has(`${p.provider}/${p.model}/${p.effort ?? ""}`), `${id}: ${p.model} ${p.effort ?? ""}`)
    }
  }
})

test("every key answers for every single provider, and for all three", () => {
  for (const id of Object.keys(DOC.keys)) {
    assert.ok(byKey(DOC.keys, id, ALL).pick, `${id} must answer with all three signed in`)
  }
  // one shortlist per key names every vendor, so nobody is ever left with nothing
  for (const id of Object.keys(DOC.keys)) {
    const named = new Set(DOC.keys[id].picks.map((p) => p.provider))
    assert.deepEqual([...named].sort(), [...ALL].sort(), `${id} must name all three vendors`)
  }
})

test("the real measured dial answers only where current models have measurements", () => {
  for (const bench of Object.keys(DOC.benches)) {
    const a = byIntelligence(DOC.models, ALL, bench, 7)
    const candidates = plottable(DOC.models.filter((m) => m.status !== "legacy" && m.status !== "restricted"), bench)
    if (candidates.length) {
      assert.ok(a.pick, `${bench} has eligible measurements`)
      assert.equal(Object.keys(a.ladder).length, 11)
      assert.ok(Object.values(a.ladder).every((pick) => candidates.some((m) => m.provider === pick.provider && m.model === pick.model && m.effort === pick.effort)))
    } else {
      assert.equal(a.pick, null, `${bench} has no current measured candidate`)
      assert.deepEqual(a.ladder, {})
    }
  }
})

test("a model with no number on a board is not a candidate there", () => {
  for (const bench of Object.keys(DOC.benches)) {
    const scored = DOC.models.filter((m) => Number.isFinite(m.scores?.[bench]) && Number.isFinite(m.price) && Number.isFinite(m.secs))
    assert.equal(plottable(DOC.models, bench).length, scored.length)
    assert.ok(scored.length > 0)
  }
})

test("a null number is not read as zero", () => {
  const sneaky = [{ provider: "codex-app-server", model: "sneaky", effort: "high", scores: { b: 1000 }, price: null, secs: null }]
  assert.deepEqual(plottable(sneaky, "b"), [])
  assert.ok(unusable(sneaky, "b").some((u) => u.entry.model === "sneaky"))
})

 test("registry rows have unique model and effort identities", () => {
  const ids = DOC.models.map((m) => `${m.provider}/${m.model}/${m.effort ?? ""}`)
  assert.equal(new Set(ids).size, ids.length)
})
