"""Test group: the adapters — turning each CLI's own wire format into omni events.

Every fixture line here was captured from a real run of the real CLI.
"""

import json

from omni.events import Event
from omni.providers.claude import session as claude_session
from omni.providers.claude.stream import Stream as ClaudeStream
from omni.providers.google.stream import Stream as AgyStream
from omni.providers.google.stream import user_line as agy_line
from omni.providers.openai import account as codex_account
from omni.providers.openai.runner import items as codex_items
from omni.providers.openai.stream import Stream as CodexStream
from omni.providers.claude.account import window as claude_window
from omni.providers.google.account import windows as agy_windows


# ----------------------------------------------------------------- claude

CLAUDE_LINES = [
    '{"type":"system","subtype":"init","session_id":"s-1","model":"claude-haiku-4-5-20251001","tools":[]}',
    '{"type":"assistant","message":{"model":"claude-haiku-4-5-20251001","content":[{"type":"thinking","thinking":"secret plan","signature":"CAIS…"}]}}',
    '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"echo hi"}}]}}',
    '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"text","text":"hi"}]}]}}',
    '{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}]}}',
    '{"is_error":false,"subtype":"success","type":"result","result":"Done.","stop_reason":"end_turn","total_cost_usd":0.004}',
]


def claude_events():
    stream = ClaudeStream()
    return [event for line in CLAUDE_LINES for event in stream.feed(line)], stream


def test_claude_turn_becomes_omni_events():
    events, stream = claude_events()
    assert [e.type for e in events] == ["thinking", "tool.call", "tool.result", "text", "end"]
    assert stream.session_id == "s-1"
    assert stream.model == "claude-haiku-4-5-20251001"


def test_claude_reasoning_is_recorded_but_never_kept():
    events, _ = claude_events()
    thinking = next(e for e in events if e.type == Event.THINKING)
    assert thinking.text == ""
    assert "secret plan" not in json.dumps(thinking.to_dict())


def test_claude_tool_result_is_named_after_its_call():
    events, _ = claude_events()
    result = next(e for e in events if e.type == Event.TOOL.RESULT)
    assert result.tool == "Bash" and result.result == "hi" and result.ok


def test_claude_failures_are_classified():
    stream = ClaudeStream()
    events = stream.feed('{"type":"result","subtype":"error_during_execution","is_error":true,"result":"429 rate limit"}')
    assert events[0].type == Event.ERROR and events[0].kind == "limit"
    assert events[1].type == Event.END


def test_claude_rate_limit_only_speaks_up_when_rejected():
    stream = ClaudeStream()
    assert stream.feed('{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}') == []
    events = stream.feed('{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour"}}')
    assert events[0].kind == "limit"


def test_claude_ignores_junk_rather_than_dying():
    stream = ClaudeStream()
    assert stream.feed("not json") == []
    assert stream.feed('{"type":"system","subtype":"thinking_tokens"}') == []


# --------------------------------------------------------- claude seeding

def test_the_project_slug_is_every_non_alphanumeric_turned_into_a_dash():
    assert claude_session.slug("/tmp/a_b.c/d") == "-tmp-a-b-c-d"


def test_a_seeded_file_is_a_chained_transcript(tmp_path):
    turns = [{"role": "user", "text": "hi"}, {"role": "assistant", "text": "hello"}]
    path = claude_session.seed(str(tmp_path), "11111111-1111-1111-1111-111111111111", turns, "m")
    records = [json.loads(line) for line in path.read_text().splitlines()]
    assert [r["type"] for r in records] == ["user", "assistant"]
    assert records[0]["parentUuid"] is None
    assert records[1]["parentUuid"] == records[0]["uuid"]
    assert all(r["timestamp"] and r["uuid"] for r in records)
    assert isinstance(records[1]["message"]["content"], list), "assistant content must be blocks"


def test_seeding_again_chains_onto_what_is_already_there(tmp_path):
    sid = "22222222-2222-2222-2222-222222222222"
    claude_session.seed(str(tmp_path), sid, [{"role": "user", "text": "one"}], "m")
    path = claude_session.seed(str(tmp_path), sid, [{"role": "user", "text": "two"}], "m")
    records = [json.loads(line) for line in path.read_text().splitlines()]
    assert len(records) == 2
    assert records[1]["parentUuid"] == records[0]["uuid"]


def test_claude_input_lines_are_shaped_the_way_the_cli_wants():
    line = json.loads(claude_session.user_line("hi"))
    assert line["type"] == "user"
    assert line["message"]["content"][0]["text"] == "hi"


# ------------------------------------------------------------------ codex

def test_codex_turn_becomes_omni_events():
    stream = CodexStream()
    out = []
    out += stream.feed("item/started", {"item": {"type": "reasoning", "id": "rs_1"}})
    out += stream.feed("item/started", {"item": {"type": "commandExecution", "id": "c1", "command": "echo hi"}})
    out += stream.feed("item/completed", {"item": {"type": "commandExecution", "id": "c1", "aggregatedOutput": "hi\n", "exitCode": 0, "status": "completed"}})
    out += stream.feed("item/completed", {"item": {"type": "agentMessage", "id": "m1", "text": "Done.", "phase": "final_answer"}})
    out += stream.feed("turn/completed", {"turn": {"status": "completed", "durationMs": 12}})
    assert [e.type for e in out] == ["thinking", "tool.call", "tool.result", "text", "end"]
    assert out[1].tool == "shell" and out[1].args == {"command": "echo hi"}
    assert out[2].result == "hi\n" and out[2].ok


def test_an_unknown_codex_tool_still_shows_up_as_a_tool():
    """New Codex item types must work the day they ship, with no change here."""
    stream = CodexStream()
    started = stream.feed("item/started", {"item": {"type": "somethingBrandNew", "id": "x1", "query": "kites"}})
    done = stream.feed("item/completed", {"item": {"type": "somethingBrandNew", "id": "x1", "answer": "42"}})
    assert started[0].type == Event.TOOL.CALL and started[0].tool == "somethingBrandNew"
    assert started[0].args == {"query": "kites"}
    assert done[0].type == Event.TOOL.RESULT


def test_the_users_own_message_is_not_echoed_back_into_history():
    stream = CodexStream()
    assert stream.feed("item/completed", {"item": {"type": "userMessage", "id": "u1"}}) == []


def test_a_failed_codex_turn_reports_why_then_ends():
    stream = CodexStream()
    out = stream.feed("turn/completed", {"turn": {"status": "failed", "error": {"message": "usageLimitExceeded"}}})
    assert out[0].type == Event.ERROR and out[0].kind == "limit"
    assert out[1].type == Event.END


def test_history_becomes_codex_response_items():
    out = codex_items([{"role": "user", "text": "hi"}, {"role": "assistant", "text": "yo"}])
    assert out[0] == {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
    assert out[1]["content"][0]["type"] == "output_text"


def test_codex_windows_are_read_by_duration_never_by_position():
    payload = {
        "rateLimits": {"primary": {"usedPercent": 16, "windowDurationMins": 10080, "resetsAt": 99}, "secondary": None},
        "rateLimitsByLimitId": {
            "other": {"primary": {"usedPercent": 4, "windowDurationMins": 300, "resetsAt": 55}}
        },
    }
    assert codex_account.windows(payload) == {
        "7d": {"used": 0.16, "reset": "1970-01-01T00:01:39.000Z"},
        "5h": {"used": 0.04, "reset": "1970-01-01T00:00:55.000Z"},
    }


# -------------------------------------------------------------------- agy

AGY_LINES = [
    '{"event":"init","conversation_id":"c-9","init":{"tools":[]}}',
    '{"event":"step_update","step_update":{"step_index":0,"state":"DONE","step_type":"user_input"}}',
    '{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"agent_response","text_delta":"He"}}',
    '{"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"agent_response","text_delta":"llo","usage":{"thinking_tokens":12}}}',
    '{"event":"step_update","step_update":{"step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"parameters":{"CommandLine":"echo hi"}}}}',
    '{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"parameters":{"CommandLine":"echo hi"},"output":"hi\\n"}}}',
    '{"event":"result","result":{"conversation_id":"c-9","status":"SUCCESS","response":"Hello","num_turns":1}}',
]


def test_agy_turn_becomes_omni_events():
    stream = AgyStream()
    out = [event for line in AGY_LINES for event in stream.feed(line)]
    assert [e.type for e in out] == ["thinking", "text", "tool.call", "tool.result", "end"]
    assert stream.conversation == "c-9"


def test_agy_text_deltas_are_stitched_back_together():
    stream = AgyStream()
    out = [event for line in AGY_LINES for event in stream.feed(line)]
    assert next(e for e in out if e.type == Event.TEXT).text == "Hello"


def test_an_agy_answer_that_never_streamed_still_arrives():
    stream = AgyStream()
    out = stream.feed('{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"OK"}}')
    assert [e.type for e in out] == ["text"] and out[0].text == "OK"


def test_a_tool_that_never_announced_itself_still_pairs_up():
    stream = AgyStream()
    out = stream.feed('{"event":"step_update","step_update":{"step_index":4,"state":"DONE","step_type":"tool","tool_name":"grep_search","tool_info":{"parameters":{"Query":"x"},"output":"none"}}}')
    assert [e.type for e in out] == ["tool.call", "tool.result"]


def test_an_agy_tool_error_is_not_reported_as_success():
    stream = AgyStream()
    out = stream.feed('{"event":"step_update","step_update":{"step_index":5,"state":"ERROR","step_type":"tool","tool_name":"run_command","tool_info":{"error":{"message":"denied"}}}}')
    result = next(e for e in out if e.type == Event.TOOL.RESULT)
    assert not result.ok and result.result == "denied"


def test_agy_input_uses_event_not_type():
    line = json.loads(agy_line("hi"))
    assert line["event"] == "user" and line["message"]["content"] == "hi"


def test_agy_reports_remaining_but_omni_reports_used():
    stream = json.dumps(
        {
            "event": "command_result",
            "command": {
                "name": "usage",
                "data": {
                    "groups": [
                        {"buckets": [{"window": "5h", "remaining_fraction": 0.9, "reset_time": 1755600000}]},
                        {"buckets": [{"window": "weekly", "remaining_fraction": 1.0, "reset_time": 1755686400}]},
                    ]
                },
            },
        }
    )
    assert agy_windows(stream) == {
        "5h": {"used": 0.1, "reset": "2025-08-19T10:40:00.000Z"},
        "7d": {"used": 0.0, "reset": "2025-08-20T10:40:00.000Z"},
    }


def test_claude_utilisation_is_rescaled_to_a_fraction():
    entry = {"utilization": 24, "resets_at": "2026-08-20T10:00:00+05:30"}
    assert claude_window(entry) == {"used": 0.24, "reset": "2026-08-20T04:30:00.000Z"}
    assert claude_window(None) == {"used": None, "reset": None}


# ------------------------------------------------------- one clock for all

def test_every_provider_says_when_the_same_way():
    """The spec asks for ISO strings from all three, whatever they answer in."""
    from omni.shared.clock import iso

    resets = [
        iso(1755600000),                    # agy: seconds
        iso(1755600000000),                 # and milliseconds, if it ever changes
        iso("2025-08-19T10:40:00Z"),        # claude: already a string
        iso("2025-08-19T16:10:00+05:30"),   # codex: a string with an offset
    ]
    assert resets == ["2025-08-19T10:40:00.000Z"] * 4


def test_a_time_nobody_can_parse_is_handed_back_rather_than_invented():
    from omni.shared.clock import iso

    assert iso("some time next week") == "some time next week"
    assert iso(None) is None and iso("") is None


def test_a_working_directory_is_resolved_before_anything_is_named_after_it(tmp_path):
    """macOS links /var and /tmp; an unresolved cwd sends Claude's file somewhere else."""
    from omni.providers.base import Config

    real = tmp_path / "real"
    real.mkdir()
    link = tmp_path / "link"
    link.symlink_to(real)
    assert Config(cwd=str(link)).cwd == str(real.resolve())


def test_the_codex_jail_holds_only_a_credentials_link_and_a_config(monkeypatch, tmp_path):
    """Two things in the folder, so codex has nothing left to auto-load."""
    from omni.providers.openai import jail

    real = tmp_path / "codex"
    real.mkdir()
    (real / "auth.json").write_text("{}")
    (real / "config.toml").write_text("[mcp_servers.thing]\ncommand='x'\n")
    monkeypatch.setattr(jail, "real_home", lambda: real)

    isolated = jail.build("s")
    assert sorted(p.name for p in isolated.iterdir()) == ["auth.json", "config.toml"]
    assert (isolated / "auth.json").is_symlink(), "linked so a token refresh is not lost"
    assert "mcp_servers" not in (isolated / "config.toml").read_text()


def test_a_jail_survives_its_credentials_link_going_stale(monkeypatch, tmp_path):
    from omni.providers.openai import jail

    real = tmp_path / "codex"
    real.mkdir()
    (real / "auth.json").write_text("{}")
    monkeypatch.setattr(jail, "real_home", lambda: real)
    isolated = jail.build("s")

    (real / "auth.json").unlink()  # the link now points at nothing
    jail.build("s")
    assert not (isolated / "auth.json").exists()
    (real / "auth.json").write_text("{}")
    assert (jail.build("s") / "auth.json").is_symlink()


def test_a_yes_is_remembered_longer_than_a_no(monkeypatch):
    """Probing costs seconds; a network blip must not drop a provider off the dial."""
    from omni.providers.base import Account
    from omni.shared import clock

    answers = ["authenticated"]
    now = [1000.0]
    monkeypatch.setattr(clock, "epoch", lambda: now[0])

    class Flaky(Account):
        def probe(self):
            return answers[-1]

    account = Flaky()
    assert account.auth_status == "authenticated"
    answers.append("unauthenticated")
    now[0] += 30
    assert account.auth_status == "authenticated", "a yes is trusted for a minute"
    now[0] += 40
    assert account.auth_status == "unauthenticated"
    answers.append("authenticated")
    now[0] += 15
    assert account.auth_status == "authenticated", "a no is re-checked quickly"
