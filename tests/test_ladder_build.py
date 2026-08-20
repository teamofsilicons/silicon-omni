"""Test group: the ladder generator — picking the leftmost models off the graph.

``tools/build_ladder.py`` is what turns Artificial Analysis' GDPval-AA v2 graph
into the finished dial omni ships. It is not part of the package, but it decides
what every user's dial contains, so its two decisions are worth pinning down.
"""

from tools.build_ladder import dial, edge

GRAPH = [
    {"provider": "a", "model": "a-top", "score": 1800, "price": 30.0},
    {"provider": "b", "model": "b-good", "score": 1600, "price": 8.0},
    {"provider": "a", "model": "a-dear", "score": 1500, "price": 12.0},
    {"provider": "b", "model": "b-mid", "score": 1300, "price": 3.0},
    {"provider": "a", "model": "a-floor", "score": 900, "price": 0.5},
]


def test_a_model_nothing_beats_on_both_axes_stays_on_the_edge():
    assert [p["model"] for p in edge(GRAPH)] == ["a-top", "b-good", "b-mid", "a-floor"]


def test_a_model_that_is_worse_and_dearer_is_never_offered():
    """a-dear costs more than b-good and scores lower: you would never pick it."""
    assert "a-dear" not in [p["model"] for p in edge(GRAPH)]


def test_the_edge_runs_down_and_to_the_left():
    walk = edge(GRAPH)
    assert all(walk[i]["score"] > walk[i + 1]["score"] for i in range(len(walk) - 1))
    assert all(walk[i]["price"] > walk[i + 1]["price"] for i in range(len(walk) - 1))


def test_losing_a_provider_puts_a_shadowed_model_back_on_the_dial():
    alone = [p["model"] for p in edge([p for p in GRAPH if p["provider"] == "a"])]
    assert alone == ["a-top", "a-dear", "a-floor"], "b-good was the only thing beating a-dear"


def test_two_models_on_the_same_spot_count_once():
    twins = GRAPH + [{"provider": "b", "model": "b-twin", "score": 900, "price": 0.5}]
    spots = [(p["score"], p["price"]) for p in edge(twins)]
    assert len(spots) == len(set(spots))


def test_the_dial_puts_the_best_at_ten_and_the_cheapest_at_zero():
    rungs = dial(edge(GRAPH))
    assert sorted(rungs) == sorted(str(i) for i in range(11))
    assert rungs["10"]["model"] == "a-top"
    assert rungs["0"]["model"] == "a-floor"


def test_a_short_edge_still_fills_every_level():
    rungs = dial(edge(GRAPH)[:2])
    assert len(rungs) == 11
    assert {r["model"] for r in rungs.values()} == {"a-top", "b-good"}


def test_a_single_model_fills_the_whole_dial():
    rungs = dial([GRAPH[0]])
    assert len(rungs) == 11 and {r["model"] for r in rungs.values()} == {"a-top"}


def test_nothing_plottable_means_no_dial():
    assert dial([]) == {}
