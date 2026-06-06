import json
from pathlib import Path

from staleness_spike.report import write_reports, GO_BAR_FP, GO_BAR_RECALL
from staleness_spike.types import Label


def _rows():
    # s_good: perfect. s_bad: flags every valid case stale.
    rows = []
    for cat, actual in [("reformat", Label.VALID), ("change-logic", Label.STALE)]:
        rows.append({"memory_id": "m", "category": cat, "strategy": "s_good",
                     "predicted_stale": actual == Label.STALE, "actual": actual.value,
                     "relocated_to": None})
        rows.append({"memory_id": "m", "category": cat, "strategy": "s_bad",
                     "predicted_stale": True, "actual": actual.value, "relocated_to": None})
    return rows


def test_write_reports_emits_both_files(tmp_path):
    write_reports(_rows(), tmp_path)
    assert (tmp_path / "scoreboard.md").exists()
    assert (tmp_path / "raw.jsonl").exists()


def test_raw_jsonl_has_one_line_per_row(tmp_path):
    rows = _rows()
    write_reports(rows, tmp_path)
    lines = (tmp_path / "raw.jsonl").read_text().splitlines()
    assert len(lines) == len(rows)
    assert json.loads(lines[0])["strategy"] in {"s_good", "s_bad"}


def test_scoreboard_marks_go_and_nogo(tmp_path):
    write_reports(_rows(), tmp_path)
    board = (tmp_path / "scoreboard.md").read_text()
    assert "s_good" in board and "s_bad" in board
    assert "GO" in board  # s_good clears the bar
