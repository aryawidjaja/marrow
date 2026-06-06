from staleness_spike.scoring import Tally, metrics
from staleness_spike.types import Label


def test_metrics_perfect_classifier():
    t = Tally(tp=10, fp=0, tn=10, fn=0)
    m = metrics(t)
    assert m["fp_rate"] == 0.0
    assert m["recall"] == 1.0
    assert m["precision"] == 1.0
    assert m["f1"] == 1.0


def test_fp_rate_is_fp_over_actual_valid():
    # 4 valid cases, 1 wrongly flagged stale -> fp_rate 0.25
    t = Tally(tp=3, fp=1, tn=3, fn=0)
    m = metrics(t)
    assert m["fp_rate"] == 0.25


def test_recall_is_tp_over_actual_stale():
    t = Tally(tp=8, fp=0, tn=5, fn=2)  # 10 actual stale, caught 8
    assert metrics(t)["recall"] == 0.8


def test_metrics_handle_empty_tally():
    m = metrics(Tally())
    assert m["fp_rate"] == 0.0
    assert m["recall"] == 0.0
    assert m["precision"] == 0.0
    assert m["f1"] == 0.0


def test_tally_add_increments_correct_cell():
    t = Tally()
    t.add(predicted_stale=True, actual=Label.STALE)   # tp
    t.add(predicted_stale=True, actual=Label.VALID)   # fp
    t.add(predicted_stale=False, actual=Label.VALID)  # tn
    t.add(predicted_stale=False, actual=Label.STALE)  # fn
    assert (t.tp, t.fp, t.tn, t.fn) == (1, 1, 1, 1)
