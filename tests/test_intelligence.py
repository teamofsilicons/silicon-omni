"""Test group: the 0-10 dial — lookup per provider set, caching, and fallback.

The dial arrives finished. omni does not decide which models are on it; it looks
up the one for the providers it has. Choosing the models is
``tools/build_ladder.py``'s job and is tested separately.
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


def test_a_dial_we_once_fetched_beats_the_one_we_shipped():
    registry.write_cache(["claude"], dial(rung("claude", "from-upstream", "max", 1, 1)))
    blob = json.loads(registry.cache_file().read_text())
    blob["claude"]["at"] -= registry.CACHE_TTL + 1  # stale, but real
    registry.cache_file().write_text(json.dumps(blob))
    assert registry.table(["claude"])[10]["model"] == "from-upstream"


def test_a_failed_fetch_is_not_retried_on_every_call():
    """A machine that cannot reach upstream must not pay for a lookup each time."""
    registry.table(["claude"])
    entry = json.loads(registry.cache_file().read_text())["claude"]
    assert entry["source"] == "packaged" and entry["ttl"] == registry.QUIET_TTL


def test_upstream_may_answer_with_the_dial_alone_or_wrapped(monkeypatch):
    made = dial(STRONG, CHEAP)
    for payload in (made, {"ladder": made}, {"ladders": {"a+b": made}}):
        monkeypatch.setattr(registry, "fetch", lambda name, timeout=5.0, p=payload: registry.unwrap(p, name))
        assert registry.fetch("a+b")["10"]["model"] == "a-top"


def test_providers_we_have_no_dial_for_say_so():
    try:
        registry.resolve(5, ["nobody"])
    except LookupError as complaint:
        assert "nobody" in str(complaint)
    else:
        raise AssertionError("should have refused")


# ---------------------------------------------- the dial we actually ship

def test_the_packaged_file_has_a_dial_for_every_combination():
    doc = json.loads(registry.LADDER_FILE.read_text())
    combos = ["claude", "openai", "google", "claude+openai", "claude+google",
              "google+openai", "claude+google+openai"]
    assert sorted(doc["ladders"]) == sorted(combos)
    for name in combos:
        assert sorted(doc["ladders"][name]) == sorted(str(i) for i in range(11)), name


def test_every_packaged_dial_names_only_its_own_providers():
    doc = json.loads(registry.LADDER_FILE.read_text())
    for name, rungs in doc["ladders"].items():
        assert {r["provider"] for r in rungs.values()} <= set(name.split("+")), name


def test_every_packaged_dial_goes_down_and_to_the_left():
    """The whole point: a step down is cheaper, and never better."""
    doc = json.loads(registry.LADDER_FILE.read_text())
    for name, rungs in doc["ladders"].items():
        for level in range(10):
            below, above = rungs[str(level)], rungs[str(level + 1)]
            assert below["price"] <= above["price"], (name, level)
            assert below["score"] <= above["score"], (name, level)


def test_the_packaged_file_says_what_it_left_out():
    doc = json.loads(registry.LADDER_FILE.read_text())
    assert doc["source"]["url"] and doc["source"]["benchmark"]
    assert doc["caveats"], "a reader has to be able to tell what is missing and why"


def test_the_registry_can_be_pointed_somewhere_else(monkeypatch):
    """So you can run your own, or try a deployment before it has a domain."""
    assert registry.remote() == registry.REMOTE
    monkeypatch.setenv("OMNI_REGISTRY", "http://localhost:9/api/intelligence")
    assert registry.remote() == "http://localhost:9/api/intelligence"


def test_the_default_registry_is_the_hosted_one():
    assert registry.REMOTE.endswith("/api/intelligence")
