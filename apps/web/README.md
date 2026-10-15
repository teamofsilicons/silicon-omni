# Omni web, registry, and admin

This app lives inside the silicon-omni monorepo. It serves the public website,
model-selection API, and Silicon Accounts admin editor from one Next.js deployment.

- `app/choose.json`: category, curated dial, and explicit benchmark selection.
- `app/registry.json`, `app/models.json`, `app/categories`: public catalog queries.
- `app/admin`: model and ordered category editors, staged changes and conflict handling.
- `app/auth`, `app/api/auth`, `lib/auth.ts`: Silicon Accounts sign-in and authorization.
- `app/api/admin/catalog`: authorized catalog reads and conditional publication.
- `lib/store.ts`: private, versioned S3 catalog; Vercel assumes an AWS role with OIDC.
- `models.json`: development/bootstrap seed; never overwrites a published catalog.

See [deployment](../../docs/DEPLOYMENT.md) and
[recommendations](../../docs/MODEL_RECOMMENDATIONS.md).

```sh
npm ci
npm run dev
npm test
npm run build
```

The original site was imported from `teamofsilicons/omnipotent` at `089af1b`.
Its illustrations and their provenance are retained below.

## The pictures

Every photograph and engraving on the site is quantised to one bit through the
same ordered Bayer matrix the page's dither uses, which is what makes a wood
engraving from 1846 and a photograph from 2015 read as plates from one book.

[`plates.json`](plates.json) is the manifest, the licence audit and the input to
the printing script, all at once — so a caption on the site cannot drift from the
record of where its picture came from. Public domain, PD-US, CC0 and
no-known-restrictions only.

```bash
python3 tools/plates.py                 # anything not yet printed
python3 tools/plates.py switchboard     # just one
python3 tools/plates.py --force         # all of them, again
```

The script writes `public/plates/<name>.png` and `<name>@2x.png`, dithering each
size from the original rather than resampling the larger one — downscaling a
dither is how you get moiré. It also writes the rendered size back into
`plates.json`, so every `<img>` carries width and height and the page never
reflows around a plate that has not loaded.

## The colour

One sheet of warm paper, one ink, one blue, and one gradient:

```
paper   #FFFDF9      ink     #1A1A1A      blue    #1F5FB8
ramp    #76D4F0 → #F9F987 → #FEAD75 → #F15347 → #14245F
```

A **single stop** stands for a single thing: one per vendor, and the vendor
colours are not arbitrary — the deepest for the soberest CLI, the lightest for
the one that can be told the least. The **whole ramp** stands for things working
together, which on this site means exactly three: an answer built out of three
rivals' models, sampled from the ramp in [`lib/ramp.ts`](lib/ramp.ts); the
publishing pipeline; and the page itself —
each chapter carries one stop, so reading the swatches from top to bottom is
reading the gradient.

Everything else is `#1A1A1A` at a lower alpha, which is why nothing here needs a
sixteenth grey.

