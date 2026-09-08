# Grouped live benchmark

`grouped_live.py` measures one fixed end-to-end conversation through the public
Python API:

1. Claude learns five fact tokens.
2. Codex learns five fact tokens.
3. Agy learns five fact tokens.
4. Claude must return all fifteen as one exact compact JSON array.

Every teaching prompt uses the same previously validated memory-test wording,
and every fact is ten characters.
The script configures intelligence level 0 once, freezes a one-rung dial for
each provider, disables MCP/subagents, waits for both `END` and `chat.idle`
after every turn, and stops the chat and daemon it started. It never repeats an
intelligence-setting call between turns (0.3.0 can race its metadata temp file
when that call is needlessly repeated).

## Run one trial

Run from a neutral directory, with a freshly installed wheel in an isolated
venv. Do not run the script with the repository on `PYTHONPATH`; it rejects a
source-tree import by default.

```sh
REPO=/absolute/path/to/silicon-omni
VENV=/absolute/path/to/venv-030
RUN=$(mktemp -d /tmp/omni-grouped-030.XXXXXX)
mkdir -p "$RUN/work"
cd "$RUN"
env -u PYTHONPATH "$VENV/bin/python" "$REPO/benchmarks/grouped_live.py" \
  --expected-version 0.3.0 \
  --expect-architecture in-process \
  --omni-home "$RUN/omni" \
  --workdir "$RUN/work" \
  --output "$RUN/result.json" \
  --trial 030-A1
```

For the daemon release, change the venv/version and assert `daemon`:

```sh
REPO=/absolute/path/to/silicon-omni
VENV=/absolute/path/to/current-venv
VERSION=0.5.0
RUN=$(mktemp -d /tmp/omni-grouped-current.XXXXXX)
mkdir -p "$RUN/work"
cd "$RUN"
env -u PYTHONPATH "$VENV/bin/python" "$REPO/benchmarks/grouped_live.py" \
  --expected-version "$VERSION" \
  --expected-daemon-version "$VERSION" \
  --expect-architecture daemon \
  --omni-home "$RUN/omni" \
  --workdir "$RUN/work" \
  --output "$RUN/result.json" \
  --trial current-B1
```

`--dry-run` writes the exact transcript and model choices without importing
`omni`, creating state, or launching a provider. It still writes the requested
output JSON file.

The default run does no authentication preflight, so `chat.start()` through the
first Claude `END` is a real first-use path. `--preflight` is available when a
separate authenticated-only profile is wanted, but it can launch/probe CLIs in
0.3 and start/warm the daemon in later releases. Use the same setting for both
versions and never mix the two profiles.

The default frozen models are:

| Provider | Model | Effort |
| --- | --- | --- |
| Claude | `claude-haiku-4-5-20251001` | empty |
| Codex | `gpt-6-astra` | `low` |
| Agy | `gemini-3.8-flash-low` | empty |

Override them explicitly if any CLI no longer accepts one, and use the exact
same overrides in every comparison trial.

## What the JSON means

The useful latency fields are in `summary.speed`:

- `start_to_first_claude_end_ms` includes session/provider activation even
  though 0.3 returns from `start()` asynchronously and the daemon client may do
  that work inside `start()`.
- Each provider group reports the first turn separately from the median and sum
  of turns 2–5. The first turn contains a provider handoff; turns 2–5 are the
  closest measure of a steady hot CLI.
- `send_to_activity_ms` is the first provider thinking/text/tool signal,
  `send_to_text_ms` is observable text TTFT, and `send_to_end_ms` is completion.
- `send_to_start_ms` is retained only as a diagnostic. `START` is recorded at
  different acceptance boundaries in the two architectures and is not a fair
  TTFT comparison.
- The final recall is reported separately and excluded from teaching totals.

`summary.overall.pass` requires all 16 turns, exact provider/model routing,
exact raw `ACK` replies, no tools or fatal errors, an exact parsed recall array,
and clean process shutdown. `summary.recall` also reports fact coverage and
order, so a formatting-only failure remains diagnosable.

The raw event stream includes both provider `at` timestamps and local callback
observation clocks. Process snapshots and a 100 ms descendant-process timeline
record PID, PPID, process start identity, command, and whether each provider PID
was stable across its five-turn group. Native Claude session, Codex thread, and
Agy conversation identities are retained separately; PID stability alone does
not prove conversation continuity. Usage is stored raw and normalized where a
provider exposes recognizable input/cache/output/reasoning fields. CLI version
commands run only after workload timing and cleanup, avoiding an accidental
Node/Python/filesystem warm-up before the cold path.

## Fair comparison

Use at least five fresh trials per version. Alternate version order (for
example `0.3, current, current, 0.3`) rather than running every old trial before
every new one, and compare medians plus the individual artifacts. Keep machine,
network, account, model dial, CLI versions, timeouts, and `--preflight` setting
fixed. A fresh `OMNI_HOME`, work directory, and session id are required for each
trial; `--reuse-home` deliberately opts out of that guarantee.

Provider latency and generated token counts are noisy, so do not call a single
turn a regression. First compare correctness, routing, errors, usage, and
process/native identity; then compare first-use, switch-turn, and steady-state
latencies independently.

This exact order tests whether Claude survives a full round trip and whether
Codex/Agy processes remain parked after switching away. It does not return to
Codex or Agy, so it cannot by itself prove that their second activation is hot.
