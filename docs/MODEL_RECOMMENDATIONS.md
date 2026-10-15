# Model recommendations

As of **2026-10-15**. The catalog is [apps/web/models.json](../apps/web/models.json).

Omni's category recommendations are curated defaults. They balance the intended task, provider guidance, effort, cost, and available CLI integrations. They are not claims that one model wins every comparison, and no new Omni benchmark was run for this update.

## Category defaults and fallbacks

The first signed-in provider in each shortlist wins. Provider sign-in alone does not establish access to every model on that provider.

| Category | First choice | Second choice | Third choice |
| --- | --- | --- | --- |
| `fast` | GPT-6 Luna · low · fast mode | Claude Haiku 5.5 · low | Gemini 3.8 Flash · low |
| `code` | GPT-6.1 Sol · high | Claude Opus 5.5 · high | Gemini 3.8 Flash · high |
| `design` | Claude Sonnet 5.5 · medium | GPT-6.1 Sol · high | Gemini 3.8 Flash · high |
| `research` | GPT-6 Astra · high | Claude Opus 5.5 · high | Gemini 3.8 Flash · high |
| `cost` | GPT-6 Luna · low | Claude Haiku 5.5 · low | Gemini 3.8 Flash · low |
| `general` | GPT-6.1 Sol · medium | Claude Sonnet 5.5 · medium | Gemini 3.8 Flash · medium |

Each category covers Codex, Claude Code, and Antigravity. The `fast` pick enables Codex's faster service tier; the `cost` pick does not.

### Why these defaults

- **Fast and cost: GPT-6 Luna.** OpenAI positions Luna for focused, high-volume work. Low effort suits this scope, and its standard API rates are substantially below Sol and Astra. Fast mode trades additional cost for speed. Claude Haiku 5.5 provides the same broad role on Claude Code. This is a practical default, not a measured cross-provider latency winner. [Luna specifications](https://developers.openai.com/api/docs/models/gpt-6-luna), [OpenAI pricing](https://developers.openai.com/api/docs/pricing), [Claude lineup](https://platform.claude.com/docs/en/models/overview).
- **Code and general: GPT-6.1 Sol.** Official guidance recommends Sol for complex coding and agentic workflows, with near-Astra capability at lower cost. High effort is the code default because checking changes and edge cases matters; medium is the general default to balance depth and response time. Those effort choices are Omni's policy. [Codex recommendations](https://learn.chatgpt.com/docs/models), [Sol specifications](https://developers.openai.com/api/docs/models/gpt-6.1-sol).
- **Design: Claude Sonnet 5.5.** Anthropic specifically describes improvements in interface polish and document design, alongside faster iteration on scoped work. Medium effort is also its Claude Code default. Opus 5.5 remains a useful explicit choice for larger, ambiguous design-and-build projects; the catalog does not assign Sonnet a fabricated Design Arena score. [Sonnet release](https://www.anthropic.com/claude-sonnet-5-5), [Claude Code effort guidance](https://code.claude.com/docs/en/model-config).
- **Research: GPT-6 Astra.** OpenAI explicitly positions Astra for demanding reasoning and research. High effort is a considered starting point; max remains available for tasks that justify additional time and cost. Claude Opus 5.5 is the Claude Code fallback for sustained, open-ended work. This is not a claim that Astra wins every research benchmark. [Astra specifications](https://developers.openai.com/api/docs/models/gpt-6-astra), [Opus release](https://www.anthropic.com/claude-opus-5-5).
- **Antigravity: Gemini 3.8 Flash.** It is supported across the documented Antigravity plan tiers and was present in the checked account's model list. The suffix in each CLI model ID carries its thinking level. [Antigravity model availability](https://www.antigravity.google/docs/models/), [CLI model selection](https://antigravity.google/docs/cli/headless/).

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

The installed Codex model cache, refreshed on the check date, confirms the GPT-6 and GPT-5.6 IDs. Its default for GPT-6.1 Sol is `low`, while the API documentation defaults to `medium`; Omni therefore sends an explicit effort. The Luna API supports `none`, but the local Codex model metadata does not expose it. Sol and Astra also expose Codex `ultra`, which combines maximum reasoning with delegation; no measured `ultra` row is inferred from `max`. [Sol API settings](https://developers.openai.com/api/docs/models/gpt-6.1-sol), [Luna API settings](https://developers.openai.com/api/docs/models/gpt-6-luna), [Codex model controls](https://learn.chatgpt.com/docs/models).

Claude Code 2.1.289 was installed at verification time. Its documentation supports all five listed efforts for the Claude 5.5 family and defaults these models to `medium`. Exact account access was not tested by running inference. [Claude Code model configuration](https://code.claude.com/docs/en/model-config).

`agy models` returned Gemini 3.8 Flash, Claude Opus 5.5, and Claude Sonnet 5.5 at low/medium/high, plus GPT-OSS 120B medium. Omni's category fallbacks use Gemini on Antigravity. **Gemini 4 Argon is current but was absent from this account's result**; official Antigravity docs require Ultra or Enterprise access. It is not silently substituted into a default. [Antigravity availability](https://www.antigravity.google/docs/models/).

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

All **28 imported measurement rows** keep their exact original `scores`, `price`, and `secs`. The **28 newly added rows** set those three fields to `null`. Original provenance stays in the catalog's `source` field; the October references support model metadata and recommendations, not a rerun of those older measurements. Vendor launch results use different benchmark versions, effort settings, and harnesses, so they are not copied into the existing board.

The measured numeric dial cannot infer positions for unmeasured new models. Keep its measurement semantics separate from a curated routing policy, and do not substitute API token prices for its per-task cost field. Legacy rows are retained for history and explicit selection; current category defaults never select GPT-5.6 or GPT-5.5.
