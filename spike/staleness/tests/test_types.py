from staleness_spike.types import (
    Label,
    Verdict,
    RawAnchor,
    SeededMemory,
    MutationCategory,
    ground_truth,
)


def test_verdict_carries_optional_relocation():
    v = Verdict(is_stale=False, relocated_to="rich/box.py:42")
    assert v.is_stale is False
    assert v.relocated_to == "rich/box.py:42"


def test_ground_truth_maps_categories_to_labels():
    assert ground_truth(MutationCategory.REFORMAT) == Label.VALID
    assert ground_truth(MutationCategory.RENAME_LOCAL) == Label.VALID
    assert ground_truth(MutationCategory.MOVE_SYMBOL) == Label.VALID
    assert ground_truth(MutationCategory.ADD_ADJACENT) == Label.VALID
    assert ground_truth(MutationCategory.CHANGE_LOGIC) == Label.STALE
    assert ground_truth(MutationCategory.CHANGE_SIGNATURE) == Label.STALE
    assert ground_truth(MutationCategory.DELETE_SYMBOL) == Label.STALE
    assert ground_truth(MutationCategory.CONTROL) == Label.VALID


def test_seeded_memory_holds_per_strategy_payloads():
    anchor = RawAnchor(
        id="m1", file_path="a.py", symbol="f",
        start_line=1, end_line=2, snippet="def f():\n    return 1",
    )
    mem = SeededMemory(anchor=anchor, payloads={"s1": {"hash": "abc"}})
    assert mem.anchor.symbol == "f"
    assert mem.payloads["s1"]["hash"] == "abc"
