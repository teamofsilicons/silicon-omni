# Omni deployment

The Python package and Rust workspace remain at the repository root. `apps/web`
contains the imported website, public registry API, and admin editor. They share
one Next.js deployment; `infra/registry.yml` provisions the private AWS catalog.

| Address | Purpose |
| --- | --- |
| `omni.teamofsilicons.com` | Website and documentation |
| `backend.omni.teamofsilicons.com` | Public model queries and CLI authentication |
| `admin.omni.teamofsilicons.com` | Silicon Accounts sign-in and catalog editor |

## Vercel source

The existing `omnipotent` Vercel project serves all three domains from the
`teamofsilicons/silicon-omni` repository, production branch `main`, root directory
`apps/web`. Run manual deployments from the monorepo root after `vercel link`.
The former standalone website repository is no longer the deployment source.

## Local development

```sh
npm --prefix apps/web ci
cp apps/web/.env.example apps/web/.env.local
npm run dev
cargo test --workspace
npm run test:web
npm run build
```

The seed is read-only unless `OMNI_CATALOG_BUCKET` is configured. Local AWS access
uses the normal AWS credential chain. Keep credentials and `.env.local` out of Git.

## Storage

```sh
aws cloudformation deploy --template-file infra/registry.yml \
  --stack-name omni-registry --capabilities CAPABILITY_IAM \
  --parameter-overrides VercelTeam=shubham-guptas-projects-7ecb7811 VercelProject=omnipotent \
  --region us-west-1 --no-fail-on-empty-changeset
```

The stack outputs the bucket and role ARN. Initialize a new bucket once using
`aws s3api put-object --bucket BUCKET --key catalog.json --body apps/web/models.json
--content-type application/json --if-none-match '*'`. Do not upload the seed over
an existing catalog. The bucket retains previous versions and survives stack deletion.

Set production Vercel environment values: `OMNI_CATALOG_BUCKET`,
`OMNI_AWS_REGION=us-west-1`, `AWS_ROLE_ARN`, `SILICON_APP_SECRET`,
`OMNI_ADMIN_ORIGIN=https://admin.omni.teamofsilicons.com`,
`OMNI_API_ORIGIN=https://backend.omni.teamofsilicons.com`, and `OMNI_ADMIN_IDS`.
The role trusts only this Vercel project's production workload. Do not place AWS
access keys in the project. Preview deployments use bundled data and do not receive
production secrets or write access.

## Identity and publishing

Silicon Accounts application ID: `omni`. Browser callback:
`https://admin.omni.teamofsilicons.com/auth/callback`. Device flow is enabled.
`http://localhost:3000/auth/callback` is registered for local development.

Admin access is an explicit list of immutable UUIDs in `OMNI_ADMIN_IDS`, initially
the owning `si:tos` and its custodian `c:shubham`. Account names do not grant access.
Sessions expire within 30 minutes; the client must sign in again. A user who signs
in without an allowed UUID sees an access message and cannot read admin data or write.

To add a model: sign in, add its provider/ID/effort, leave unknown measurements blank,
stage it, optionally add it to a category shortlist, and publish. Legacy and restricted
models stay available by explicit name but cannot enter recommendation shortlists.
Concurrent edits produce HTTP 409 and require loading the latest catalog.

Seven categories are required when publishing, including `writing`. Older saved
catalogs and browser drafts are upgraded on read by deriving an initial writing
shortlist from `general`; reading never overwrites the stored catalog.

Argon recommendations require the exact model ID to appear in the caller's
Antigravity inventory. Keep Flash high directly after Argon as its fallback.
The CLI probes `agy models`; the daemon reuses that inventory for at most 30
seconds and clears it when account state is forgotten. Unknown access skips Argon.

Public service responses cache for up to one minute; existing local daemons cache
selection results for one hour. No redeployment or Python release is needed to publish.
`tools/refresh.py` only refreshes the development seed; it preserves manual metadata
and never writes production data.

## API and CLI

```sh
omni categories --json
omni models --provider codex-app-server --json
omni best code --json
omni best writing --json
omni login
omni login status --json
si auth setup omni
omni admin
omni logout
```

Public GET endpoints: `/categories`, `/models` (backend host), `/models.json`,
`/registry.json`, `/choose.json?key=code`, `/health`, `/accounts`.
`/choose.json?intelligence=5` uses curated categories. Supply `bench=gdpval-aa` for
an explicit measured ranking; missing current measurements return no pick.
Admin GET/PUT `/api/admin/catalog` requires an authenticated, authorized session.
PUT accepts `{catalog, revision}` and uses that revision as an S3 ETag condition.
`/choose.json` accepts `available_models` as a JSON provider-to-model-ID map
(URL-encoded for GET), for example
`{"antigravity-cli":["gemini-4-argon-high"]}`. Both category and numeric
queries apply this evidence to picks marked `requires_availability`. Without it,
conditional picks are skipped. Explicit model requests remain explicit.
`omni best` probes on each request; recommendation cache keys include the inventory.

## Silicon Apps release

Keep `infra/silicon-apps.yaml` at the Rust workspace release version. With Zig and
`cargo-zigbuild` on `PATH`, build the Linux package from the repository root:

```sh
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu \
  -p silicon-omni-cli -p omni-daemon --bin omni --bin omnid
mkdir -p target/silicon-apps/bin
cp infra/silicon-apps.yaml target/silicon-apps/apps.yaml
cp target/x86_64-unknown-linux-gnu/release/omni \
  target/x86_64-unknown-linux-gnu/release/omnid target/silicon-apps/bin/
chmod 755 target/silicon-apps/bin/omni target/silicon-apps/bin/omnid
silicon-apps validate target/silicon-apps
silicon-apps pack target/silicon-apps --output target/omni-linux-x86_64.tar.gz
```

The package includes `omnid` beside `omni` so the CLI can start its daemon.
Authenticate Silicon Apps as an author of `omni`, then upload and use the returned
package ID and development release ID in the following commands:

```sh
silicon-apps upload omni target/omni-linux-x86_64.tar.gz --target linux-x86_64 --json
silicon-apps release omni --version 0.10.5 --package PACKAGE_ID --json
silicon-apps promote omni DEVELOPMENT_RELEASE_ID --version 0.10.5 --json
silicon-apps publish omni
```

Upload validation runs `--help`, `accounts --json`, and `login status --json` in an
isolated runner. Account discovery and signed-out status must work without network
access. For retryable publishing operations, save an `--idempotency-key` and reuse
it only for the same request; changed package bytes need a new key.

## Recovery

Use S3 object versions to inspect an older `catalog.json` after an unwanted edit.
Publish reviewed older contents through the admin API with the **current** revision
so recovery also respects concurrency and records its actor. Do not delete the bucket
or grant it public access. A production storage outage returns an error rather than
silently resurrecting a stale checked-in catalog.
