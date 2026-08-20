# the silicon omni site

The landing page, the reference, and the registry omni reads its dial from.

```
app/
  page.tsx                 landing
  reference/page.tsx       every method, event, file and endpoint
  dial/page.tsx            the dial, the whole graph, and an entry builder
  api/intelligence         the dial, as omni asks for it
  api/graph                every model, levelled or not
  api/models               litellm's catalogue, narrowed to our three vendors
lib/
  providers.ts             the three CLIs and the effort words each accepts
  dial.ts                  the left edge, and the 0-10 walk
  graph.ts                 reads models.json out of the repo
data/models.json           the models. this is the source of truth
```

## Changing the dial

Edit [`docs/data/models.json`](data/models.json) and commit. That is the whole
flow — no database, no login, no release, and no redeploy. The site reads the
file over `raw.githubusercontent.com` at request time and caches it for a
minute, so a change is live about as fast as you can refresh.

Git is doing the work a database would have: history, blame, review, rollback.

One entry per model **and** effort, because effort changes both what a model
scores and what it costs:

```json
{ "provider": "openai", "model": "gpt-5.6-luna", "effort": "max",
  "score": 1578.3, "price": 0.1022 }
```

- `provider` — which CLI runs it: `claude`, `openai` or `google`.
- `model` — passed to the CLI verbatim. It does not have to exist in any
  catalogue; `gpt-5.6-sol` and `gemini-3.7-flash-high` are CLI-only slugs.
- `effort` — must be one the CLI accepts. `""` means the flag is not passed at
  all, which Antigravity needs whenever the slug already carries the effort.
- `score` / `price` — a GDPval-AA v2 Elo, and USD per GDPval task from the same
  leaderboard. Leave either out and the model is kept but stays off the dial,
  which is the honest state for something nobody has benchmarked yet.

`/dial` has a builder that gets those three unguessable fields right and hands
you the JSON to paste. It writes nothing.

## Deploying

Import the repo on Vercel and set the project's **root directory to `docs`**.
That is all — nothing to provision and no environment variables to set. Set
`OMNI_REPO` or `OMNI_BRANCH` only if the model list should come from somewhere
other than `teamofsilicons/silicon-omni` on its default branch.

## Pointing omni at it

```bash
export OMNI_REGISTRY=https://your-deployment.vercel.app/api/intelligence
```

omni asks for the dial matching the providers it has, caches it under
`~/.omni/cache` for an hour, and prefers a dial it fetched before — even a stale
one — over the copy packaged with the release.

## The contract

```
GET /api/intelligence?providers=claude+google
```

```json
{
  "providers": "claude+google",
  "ladder":  { "10": { "provider": "claude", "model": "claude-opus-5", "effort": "max" }, "...": {} },
  "ladders": { "claude": {}, "claude+google": {}, "...": {} },
  "source":  { "benchmark": "GDPval-AA v2, Artificial Analysis", "...": "" },
  "caveats": ["..."]
}
```

omni reads `ladders[<providers>]` and falls back to `ladder`, so either shape
alone is a valid answer. Levels run `"0"` to `"10"`; `provider`, `model` and
`effort` are the only fields it needs.

## What the dial is

Every model is a point: a GDPval-AA v2 Elo, and the dollars it measurably cost
to earn that score. Only the left edge becomes a dial — a model earns a level if
nothing else is both better *and* cheaper. Level 10 is the top of the edge and
the walk goes down and to the left, so a step down is always a real saving.

There is one dial per set of providers, because losing a vendor puts models back
on the dial that another vendor's were shadowing. `lib/dial.ts` is the same
calculation `tools/build_ladder.py` does offline; the site runs it per request
over whatever is in the file.

If GitHub cannot be reached the site serves the copy compiled into the
deployment and says so, so it is never simply down.

## Local

```bash
npm install
npm run dev
```
