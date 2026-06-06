from pathlib import Path

from staleness_spike.runner import run_spike
from staleness_spike.types import Label


def test_run_spike_produces_rows_for_every_strategy(tmp_repo):
    files = {"pkg/a.py": "def f(x):\n    total = x + 1\n    return total\n",
             "pkg/b.py": "Y = 1\n"}
    repo = tmp_repo(files)
    rows = run_spike(repo, count=1, seed=7)
    strategies = {r["strategy"] for r in rows}
    assert strategies == {"s1_exact", "s2_normalized", "s3_ast", "s4_relocation"}


def test_run_spike_rows_have_required_fields(tmp_repo):
    files = {"pkg/a.py": "def f(x):\n    total = x + 1\n    return total\n",
             "pkg/b.py": "Y = 1\n"}
    repo = tmp_repo(files)
    rows = run_spike(repo, count=1, seed=7)
    r = rows[0]
    assert set(r) >= {"memory_id", "category", "strategy", "predicted_stale", "actual"}
    assert r["actual"] in {Label.STALE.value, Label.VALID.value}


def test_control_rows_are_never_predicted_stale(tmp_repo):
    files = {"pkg/a.py": "def f(x):\n    total = x + 1\n    return total\n",
             "pkg/b.py": "Y = 1\n"}
    repo = tmp_repo(files)
    rows = run_spike(repo, count=1, seed=7)
    control = [r for r in rows if r["category"] == "control"]
    assert control  # control category present
    for r in control:
        assert r["predicted_stale"] is False
