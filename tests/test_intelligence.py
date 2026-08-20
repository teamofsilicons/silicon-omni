"""Test group: the 0-10 dial — asking the registry, caching, and refusing.

The dial arrives finished. omni does not decide which models are on it and does
not know the name of a single one; it looks up the map for the providers it has.
Choosing the models happens at the registry, and is tested there.
"""

import json

from omni.intelligence import registry

from .fake import dial, rung

STRONG = rung("a", "a-top", "max", 1800, 9.0)
MIDDLE = rung("b", "b-mid", "high", 1400, 2.0)
CHEAP = rung("a", "a-floor", "low", 900, 0.2)


def test_every_level_from_zero_to_ten_resolves():
    registry.write_cache(["a", "b"], dial(STRONG, MIDDLE, CHEAP))
    rungs = registry.table(["a", "b"])
    assert sorted(rungs) == list(range(11))
    assert all({"provider", "model", "effort", "level"} <= set(r) for r in rungs.values())


def test_ten_is_the_best_you_can_reach_and_zero_the_cheapest():
    registry.write_cache(["a", "b"], dial(STRONG, MIDDLE, CHEAP))
    rungs = registry.table(["a", "b"])
    assert rungs[10]["model"] == "a-top"
    assert rungs[0]["model"] == "a-floor"


def test_each_set_of_providers_has_its_own_dial():
    """Losing a provider is a different dial, not a filtered one."""
    registry.write_cache(["a", "b"], dial(STRONG, CHEAP))
    registry.write_cache(["a"], dial(rung("a", "a-alone", "high", 1500, 5.0), CHEAP))
    assert registry.table(["a", "b"])[10]["model"] == "a-top"
    assert registry.table(["a"])[10]["model"] == "a-alone"


def test_the_order_providers_are_given_in_does_not_matter():
    registry.write_cache(["a", "b"], dial(STRONG, CHEAP))
    assert registry.table(["b", "a"]) == registry.table(["a", "b"])


def test_out_of_range_levels_clamp():
    registry.write_cache(["a", "b"], dial(STRONG, CHEAP))
    assert registry.resolve(99, ["a", "b"])["model"] == "a-top"
    assert registry.resolve(-5, ["a", "b"])["model"] == "a-floor"


# ------------------------------------------------------------------ caching

def test_a_cache_older_than_an_hour_is_ignored():
    registry.write_cache(["a", "b"], dial(STRONG, CHEAP))
    blob = json.loads(registry.cache_file().read_text())
    blob["a+b"]["at"] -= registry.CACHE_TTL + 1
    registry.cache_file().write_text(json.dumps(blob))
    assert registry.read_cache("a+b") is None


def test_a_cache_from_an_older_omni_is_ignored():
    registry.write_cache(["a", "b"], dial(STRONG, CHEAP))
    blob = json.loads(registry.cache_file().read_text())
    blob["version"] = registry.VERSION + 1
    registry.cache_file().write_text(json.dumps(blob))
    assert registry.read_cache("a+b") is None


def test_the_cache_lives_under_the_omni_home():
    from omni.shared import paths

    registry.write_cache(["a"], dial(CHEAP))
    assert registry.cache_file() == paths.cache() / "intelligence.json"
    assert registry.cache_file().is_relative_to(paths.home())


def test_a_machine_that_has_run_before_keeps_working_offline():
    """An expired answer beats no answer when the registry cannot be reached."""
    registry.write_cache(["a"], dial(STRONG, CHEAP))
    blob = json.loads(registry.cache_file().read_text())
    blob["a"]["at"] -= registry.CACHE_TTL + 1
    registry.cache_file().write_text(json.dumps(blob))
    assert registry.table(["a"])[10]["model"] == "a-top"


def test_an_unreachable_registry_is_not_retried_on_every_call():
    registry.write_cache(["a"], dial(CHEAP))
    blob = json.loads(registry.cache_file().read_text())
    blob["a"]["at"] -= registry.CACHE_TTL + 1
    registry.cache_file().write_text(json.dumps(blob))
    registry.table(["a"])
    assert json.loads(registry.cache_file().read_text())["a"]["ttl"] == registry.QUIET_TTL


# ------------------------------------------------------------------ asking

def test_the_registry_is_asked_for_exactly_the_providers_we_have(monkeypatch):
    asked = []
    monkeypatch.setattr(registry, "fetch", lambda name, timeout=5.0: asked.append(name) or dial(CHEAP))
    registry.table(["b", "a"])
    assert asked == ["a+b"], "sorted, so both sides agree on the name"


def test_a_fetched_dial_is_cached_for_an_hour(monkeypatch):
    monkeypatch.setattr(registry, "fetch", lambda name, timeout=5.0: dial(STRONG, CHEAP))
    registry.table(["a"])
    assert json.loads(registry.cache_file().read_text())["a"]["ttl"] == registry.CACHE_TTL


def test_the_registry_may_answer_with_the_dial_alone_or_wrapped():
    made = dial(STRONG, CHEAP)
    for payload in (made, {"ladder": made}, {"ladders": {"a+b": made}}):
        assert registry.unwrap(payload, "a+b")["10"]["model"] == "a-top"


def test_an_envelope_without_our_dial_in_it_is_not_mistaken_for_one():
    """Otherwise the whole api response gets cached as if it were the dial."""
    made = dial(STRONG, CHEAP)
    assert registry.unwrap({"ladders": {"someone-else": made}, "source": {}}, "a+b") is None
    assert registry.unwrap({"error": "nope"}, "a+b") is None
    assert registry.unwrap({"0": {"no": "model"}}, "a+b") is None


def test_the_registry_can_be_pointed_somewhere_else(monkeypatch):
    """So you can run your own, or try a deployment before it has a domain."""
    assert registry.remote() == registry.REGISTRY
    monkeypatch.setenv("OMNI_REGISTRY", "http://localhost:9/intelligence.json")
    assert registry.remote() == "http://localhost:9/intelligence.json"


def test_the_default_registry_is_the_hosted_json():
    assert registry.REGISTRY.endswith("/intelligence.json")


# ------------------------------------------------------------------ refusing

def test_never_having_reached_the_registry_is_an_honest_refusal():
    """No model list ships in the wheel, so there is nothing to guess with."""
    import pytest

    with pytest.raises(registry.NoDial) as refused:
        registry.resolve(5, ["nobody"])
    assert "nobody" in str(refused.value)
    assert registry.remote() in str(refused.value)


def test_the_package_carries_no_model_names_at_all():
    """A model list baked into a release is a model list that goes stale."""
    import pathlib

    import omni

    root = pathlib.Path(omni.__file__).parent
    names = ("claude-opus", "claude-sonnet", "claude-haiku", "gpt-5", "gemini-3", "gpt-oss")
    for path in list(root.rglob("*.py")) + list(root.rglob("*.json")):
        body = path.read_text()
        for name in names:
            assert name not in body, f"{path.relative_to(root)} names a model"
