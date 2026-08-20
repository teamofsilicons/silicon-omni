# the silicon omni site

The landing page, the reference, and the registry omni reads its dial from.

```
app/
  page.tsx                 landing
  reference/page.tsx       every method, event, file and endpoint
  dial/page.tsx            add and remove models, behind a token
  api/intelligence         the dial, as omni asks for it
  api/rungs                the models on the graph
  api/models               litellm's catalogue, narrowed to our three vendors
lib/
  providers.ts             the three CLIs and the effort words each accepts
  dial.ts                  the left edge, and the 0-10 walk
  db.ts                    one table, or the seed when there is no database
data/seed.json             the graph as shipped, written by tools/build_ladder.py
```

## Deploying

1. **Import the repo** on Vercel and set the project's **root directory to `docs`**.
   Everything else is auto-detected.

2. **Add a Postgres store** (Storage → Neon). `DATABASE_URL` is injected for you;
   `POSTGRES_URL` is read too, so any Postgres works. The table is created on first
   use — there is no migration step.

3. **Set `ADMIN_TOKEN`** to something long. Anyone holding it can change what every
   omni install resolves to, so treat it like a deploy key.

4. **Open `/dial`**, paste the token, and press *plant the seed*. That copies the
   33 models in `data/seed.json` into the database. From then on the site is the
   source of truth and the seed is only a starting point.

Without a database the site still builds, still serves the dial from the seed, and
says so on `/dial`. A fresh clone is never broken; it just cannot be edited.

## Pointing omni at it

```bash
export OMNI_REGISTRY=https://your-deployment.vercel.app/api/intelligence
```

omni asks for the dial matching the providers it has, caches it under `~/.omni/cache`
for an hour, and prefers a dial it fetched before — even a stale one — over the copy
packaged with the release. Leave the variable unset and it uses
`https://omni.teamofsilicons.com/api/intelligence`.

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

omni reads `ladders[<providers>]` and falls back to `ladder`, so either shape alone
is a valid answer. Levels run `"0"` to `"10"`; `provider`, `model` and `effort` are
the only fields it needs, and `effort: ""` means the flag is not passed at all.

## What the dial is

Every model is a point: a GDPval-AA v2 Elo, and the dollars it measurably cost to
earn that score. Only the left edge becomes a dial — a model earns a level if
nothing else is both better *and* cheaper. Level 10 is the top of the edge and the
walk goes down and to the left, so a step down is always a real saving.

There is one dial per set of providers, because losing a vendor puts models back on
the dial that another vendor's were shadowing. `lib/dial.ts` is the same calculation
`tools/build_ladder.py` does offline; the site runs it per request over the database.

A model with no Elo or no cost is stored but stays off the dial. That is deliberate —
you can record a model the day it ships and fill the numbers in when they exist.

## Local

```bash
npm install
npm run dev
```

`/dial` will say there is no database and refuse to save, which is the correct
behaviour. To exercise the whole loop, set `DATABASE_URL` and `ADMIN_TOKEN` in
`.env.local` first.
