from staleness_spike.strategies import STRATEGIES
from staleness_spike.strategies import s1_exact, s2_normalized, s3_ast, s4_relocation  # noqa: F401


def test_registry_holds_four_strategy_instances():
    names = sorted(s.name for s in STRATEGIES)
    assert names == ["s1_exact", "s2_normalized", "s3_ast", "s4_relocation"]
    for s in STRATEGIES:
        assert not isinstance(s, type)  # instances, not classes
        assert callable(s.seed) and callable(s.check)
