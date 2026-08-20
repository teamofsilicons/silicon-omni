"""Test group: isolation — the exact flags that keep providers from bringing their own help.

The spec makes these invocations the contract, so they are asserted directly
rather than inferred from behaviour::

    CLAUDE_CODE_DISABLE_WORKFLOWS=1 claude ... --disallowedTools "Agent(*)"
    codex app-server --stdio --disable apps --disable plugins -c agents.enabled=false -c project_doc_max_bytes=0
    agy has no switch for any of it
"""

from omni.events import Event
from omni.providers.base import Config
from omni.providers.claude.runner import Runner as Claude
from omni.providers.google.runner import Runner as Agy
from omni.providers.openai.runner import Runner as Codex


def build(runner_cls, **config):
    caught = []
    return runner_cls("s", Config(**config), caught.append), caught


def pairs(argv):
    """Flags with their values, so ordering does not make a test brittle."""
    return {argv[i]: argv[i + 1] if i + 1 < len(argv) else "" for i in range(len(argv))}


# ------------------------------------------------------------------ claude

def test_claude_always_streams_both_ways_and_skips_permissions():
    runner, _ = build(Claude)
    argv = runner.argv(resume=False)
    assert pairs(argv)["--output-format"] == "stream-json"
    assert pairs(argv)["--input-format"] == "stream-json"
    assert "--dangerously-skip-permissions" in argv
    assert "--disable-slash-commands" in argv
    assert "--verbose" in argv


def test_claude_subagents_go_off_by_flag_and_environment():
    runner, _ = build(Claude, disable_subagents=True)
    argv = runner.argv(resume=False)
    assert pairs(argv)["--disallowedTools"] == "Agent(*)"
    assert runner.environment()["CLAUDE_CODE_DISABLE_WORKFLOWS"] == "1"


def test_claude_mcp_goes_off_with_settings_sources():
    runner, _ = build(Claude, disable_mcp=True)
    argv = runner.argv(resume=False)
    assert "--strict-mcp-config" in argv
    assert pairs(argv)["--setting-sources"] == ""


def test_claude_leaves_them_alone_unless_asked():
    runner, _ = build(Claude)
    argv = runner.argv(resume=False)
    assert "--disallowedTools" not in argv and "--strict-mcp-config" not in argv
    assert "CLAUDE_CODE_DISABLE_WORKFLOWS" not in runner.environment()


def test_claude_memory_files_are_always_off():
    """Not a switch: a run has to mean the same thing on anyone's machine."""
    env = build(Claude)[0].environment()
    assert env["CLAUDE_CODE_DISABLE_CLAUDE_MDS"] == "1"
    assert env["CLAUDE_CODE_DISABLE_AUTO_MEMORY"] == "1"
    assert env["CLAUDE_CODE_DISABLE_ORG_MEMORY"] == "1"


def test_claude_resumes_a_session_it_has_and_names_one_it_does_not():
    runner, _ = build(Claude)
    runner.native_id = "u-1"
    assert pairs(runner.argv(resume=True))["--resume"] == "u-1"
    assert pairs(runner.argv(resume=False))["--session-id"] == "u-1"


def test_claude_passes_model_effort_and_prompts_straight_through():
    runner, _ = build(Claude, model="some-future-model", effort="xhigh", system_prompt="be terse")
    argv = pairs(runner.argv(resume=False))
    assert argv["--model"] == "some-future-model"
    assert argv["--effort"] == "xhigh"
    assert argv["--system-prompt"] == "be terse"


def test_an_absent_effort_is_not_passed_at_all():
    """Some models reject the flag outright, so empty has to mean absent."""
    argv = build(Claude, model="m", effort="")[0].argv(resume=False)
    assert "--effort" not in argv


# ------------------------------------------------------------------- codex

def test_codex_subagents_apps_plugins_and_project_docs_go_off_together():
    """The spec makes this exact invocation the contract."""
    assert build(Codex, disable_subagents=True)[0].flags() == [
        "--disable", "apps",
        "--disable", "plugins",
        "-c", "agents.enabled=false",
        "-c", "project_doc_max_bytes=0",
    ]


def test_codex_mcp_is_already_gone_with_the_jail():
    assert build(Codex, disable_mcp=True)[0].flags() == []


def test_codex_is_left_alone_unless_asked():
    assert build(Codex)[0].flags() == []


def test_codex_turns_run_without_asking_permission():
    runner, _ = build(Codex, model="gpt-future", system_prompt="be terse")
    settings = runner.settings()
    assert settings["approvalPolicy"] == "never"
    assert settings["sandbox"] == "danger-full-access"
    assert settings["model"] == "gpt-future"
    assert settings["baseInstructions"] == "be terse"


# --------------------------------------------------------------------- agy

def test_agy_streams_both_ways_and_never_times_out():
    argv = build(Agy)[0].argv("")
    assert pairs(argv)["--output-format"] == "stream-json"
    assert pairs(argv)["--print-timeout"] == "24h"
    assert "--dangerously-skip-permissions" in argv


def test_agy_gives_print_an_explicit_empty_value_last():
    """--print takes a value; anywhere but last it swallows the next flag."""
    argv = build(Agy)[0].argv("")
    assert argv[-2:] == ["--print", ""]


def test_agy_resumes_by_conversation_id():
    assert pairs(build(Agy)[0].argv("c-1"))["--conversation"] == "c-1"


def test_agy_says_out_loud_what_it_cannot_switch_off():
    runner, caught = build(Agy, disable_subagents=True, disable_mcp=True)
    runner.announce()
    said = [e for e in caught if e.type == Event.CONFIG and e.text == "unsupported"]
    assert said and said[0].extra["ignored"] == ["disable_subagents", "disable_mcp"]


def test_agy_stays_quiet_when_nothing_was_asked_of_it():
    runner, caught = build(Agy)
    runner.announce()
    assert not caught


def test_agy_carries_a_system_prompt_in_as_text_and_admits_it():
    runner, caught = build(Agy, system_prompt="be terse")
    opening = runner.opening([])
    assert "be terse" in opening
    assert [e.extra for e in caught if e.text == "approximated"], "it has to say so"


def test_agy_puts_the_prompt_before_the_history():
    from omni.events import Event as E

    runner, _ = build(Agy, system_prompt="be terse")
    opening = runner.opening([E(type=E.START, text="earlier question")])
    assert opening.index("be terse") < opening.index("earlier question")
