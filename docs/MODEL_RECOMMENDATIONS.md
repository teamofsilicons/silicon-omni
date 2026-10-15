# Model recommendations

As of **2026-10-15**. The catalog is [apps/web/models.json](../apps/web/models.json).

Omni's category recommendations are configured priorities for seven kinds of work. The ordering and effort levels below are deliberate routing choices. They are not claims that one model wins every comparison, and no new Omni benchmark was run for this update.

## Category defaults and fallbacks

The first eligible pick on a signed-in provider wins. Argon needs a positive model-availability result from Antigravity; when that result is absent or does not include Argon, the next pick is Gemini 3.8 Flash high. The Argon/Flash pair occupies one provider position, preserving the priority between providers.

| Category | First choice | Second choice | Third choice |
| --- | --- | --- | --- |
| `fast` | GPT-6.1 Sol · high · fast mode | Claude Sonnet 5.5 · medium · normal speed* | Gemini 3.8 Flash · high |
| `code` | Claude Opus 5.5 · max | GPT-6 Astra · max | Gemini 4 Argon · high if available, otherwise Gemini 3.8 Flash · high |
| `design` | Claude Opus 5.5 · max | Gemini 4 Argon · high if available, otherwise Gemini 3.8 Flash · high | GPT-6 Astra · max |
| `research` | GPT-6 Astra · max | Claude Opus 5.5 · max | Gemini 4 Argon · high if available, otherwise Gemini 3.8 Flash · high |
| `cost` | GPT-6.1 Sol · low | Claude Sonnet 5.5 · low | Gemini 3.8 Flash · high |
| `general` | Claude Sonnet 5.5 · max | GPT-6.1 Sol · max | Gemini 4 Argon · high if available, otherwise Gemini 3.8 Flash · high |
| `writing` | Claude Sonnet 5.5 · max | GPT-6.1 Sol · max | Gemini 4 Argon · high if available, otherwise Gemini 3.8 Flash · high |

\* **Sonnet does not support Claude Code fast mode.** The fast shortlist keeps Sonnet 5.5 medium at normal speed. Enabling fast mode on an unsupported model changes it to Opus, so the catalog does not label Sonnet as fast or silently substitute Opus. Supported Claude fast models are Opus 5.5, Opus 5, and Opus 4.8. [Claude Code fast-mode support](https://code.claude.com/docs/en/fast-mode).

### Effort and availability

- `fast` uses Sol high with its faster service tier; `cost` uses Sol low without that tier. Their names describe routing policy, not measured cross-provider latency or cost winners.
- `code`, `design`, and `research` prioritize Opus or Astra at max as shown. `general` and `writing` prioritize Sonnet max, then Sol max. Explicit max is intentional and can consume more time and credits than lower effort.
- Every Gemini shortlist choice uses high effort, encoded in its CLI model ID. Argon's exact ID is `gemini-4-argon-high`; the CLI also documents `gemini-4-argon-medium`, which is not used in these shortlists. [Antigravity CLI model selection](https://www.antigravity.google/docs/cli/headless/#select-a-model-effort-or-agent).
- Argon is a current model requiring Google AI Ultra or Enterprise access. Its catalog row and picks set `requires_availability: true`. Clients report available model IDs by provider, and the selector skips a gated pick without positive evidence for that exact ID. Each Argon entry is immediately followed by `gemini-3.8-flash-high`. [Antigravity model availability](https://www.antigravity.google/docs/models/).

## Exact IDs and effort settings

| Provider | Model ID | Catalog efforts |
| --- | --- | --- |
| Codex | `gpt-6.1-sol` | low, medium, high, xhigh, max |
| Codex | `gpt-6-luna` | low, medium, high, xhigh, max |
| Codex | `gpt-6-astra` | low, medium, high, xhigh, max |
| Claude Code | `claude-opus-5-5` | low, medium, high, xhigh, max |
| Claude Code | `claude-sonnet-5-5` | low, medium, high, xhigh, max |
| Claude Code | `claude-haiku-5-5` | low, medium, high, xhigh, max |
| Antigravity | `gemini-3.8-flash-low`, `gemini-3.8-flash-medium`, `gemini-3.8-flash-high` | empty separate effort; encoded in ID |
| Antigravity | `gemini-4-argon-high` | empty separate effort; high encoded in ID; availability required |

The installed Codex model cache, refreshed on the check date, confirms the GPT-6 and GPT-5.6 IDs. Its default for GPT-6.1 Sol is `low`, while the API documentation defaults to `medium`; Omni therefore sends an explicit effort. The Luna API supports `none`, but the local Codex model metadata does not expose it. Sol and Astra also expose Codex `ultra`, which combines maximum reasoning with delegation; no measured `ultra` row is inferred from `max`. [Sol API settings](https://developers.openai.com/api/docs/models/gpt-6.1-sol), [Luna API settings](https://developers.openai.com/api/docs/models/gpt-6-luna), [Codex model controls](https://learn.chatgpt.com/docs/models).

Claude Code 2.1.289 was installed at verification time. Its documentation supports all five listed efforts for the Claude 5.5 family and defaults these models to `medium`. Exact account access was not tested by running inference. [Claude Code model configuration](https://code.claude.com/docs/en/model-config).

`agy models` returned Gemini 3.8 Flash, Claude Opus 5.5, and Claude Sonnet 5.5 at low/medium/high, plus GPT-OSS 120B medium. **Gemini 4 Argon was absent from this account's result**. The documented Argon high ID remains in the catalog for eligible accounts; this account selects the adjacent Flash high fallback. Provider sign-in alone is insufficient evidence of Argon access. [Antigravity availability](https://www.antigravity.google/docs/models/).

## API pricing is separate from benchmark cost

These are first-party standard API USD prices per one million tokens, checked on the date above. They are not CLI subscription prices or measured cost per completed task.

| Model | Input | Output | Scope |
| --- | ---: | ---: | --- |
| GPT-6.1 Sol | $2.00 | $10.00 | Up to 272K input tokens |
| GPT-6 Luna | $0.10 | $0.50 | Up to 272K input tokens |
| GPT-6 Astra | $10.00 | $50.00 | Up to 272K input tokens |
| Claude Opus 5.5 | $4.00 | $20.00 | Standard API |
| Claude Sonnet 5.5 | $2.00 | $10.00 | Standard API |
| Claude Haiku 5.5 | $0.10 | $0.50 | Up to 100K input tokens |
| Gemini 3.8 Flash | $0.75 | $3.75 | Introductory price through the end of 2026 |

For the GPT-6 models, requests above 272K input tokens have double input/cache rates and 1.5× output rates. Haiku 5.5 requests above 100K input tokens cost $0.50 input and $2.50 output per million tokens. Caching, faster service tiers, and regional routing have additional rules. The current Claude pricing page lists Sonnet 5.5 cache reads at $0.10 per million tokens, superseding the $0.20 figure on its launch page. [OpenAI pricing](https://developers.openai.com/api/docs/pricing), [Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing), [Gemini pricing and features](https://ai.google.dev/gemini-api/docs/generate-content/whats-new-gemini-3.8).

## Legacy options and measurement policy

`status: "legacy"` means retained for explicit selection, not recommended by the current category shortlists. It does not by itself mean the provider has discontinued a model. GPT-6 Sol and all three GPT-5.6 variants remain in the catalog; older measured GPT and Claude rows remain intact. GPT-5.6 availability during the rollout is documented by OpenAI. [Codex model availability](https://learn.chatgpt.com/docs/models).

GPT-5.5 retired from **ChatGPT-authenticated Codex** on October 14, 2026 at 19:00 UTC, or October 15 at 00:30 IST. The OpenAI API and API-key-authenticated Codex sessions are unaffected. Its retained rows carry that access note and are excluded from category recommendations. [Retirement notice](https://learn.chatgpt.com/docs/models#gpt-55-retirement).

Gemini 3.6 Flash's historical row is marked `restricted` because current Antigravity docs list Enterprise-only access. Gemini 3.5 Flash's historical row is `legacy`; neither appeared in the checked account's current model list. [Antigravity model availability](https://www.antigravity.google/docs/models/).

All **28 imported measurement rows** keep their exact original `scores`, `price`, and `secs`. The **29 newly added rows**, including Argon high, set those three fields to `null`. The catalog contains 57 configurations across 22 model IDs. Original provenance stays in the catalog's `source` field; the October references support model metadata and recommendations, not a rerun of those older measurements. Vendor launch results use different benchmark versions, effort settings, and harnesses, so they are not copied into the existing board.

The measured numeric dial cannot infer positions for unmeasured new models. Keep its measurement semantics separate from a curated routing policy, and do not substitute API token prices for its per-task cost field. Legacy rows are retained for history and explicit selection; current category defaults never select GPT-5.6 or GPT-5.5.
