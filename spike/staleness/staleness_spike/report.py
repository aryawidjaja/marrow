from __future__ import annotations

import json
from collections import defaultdict
from pathlib import Path

from staleness_spike.scoring import Tally, metrics
from staleness_spike.types import Label

GO_BAR_FP = 0.05
GO_BAR_RECALL = 0.80


def write_reports(rows: list[dict], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    _write_raw(rows, out_dir / "raw.jsonl")
    _write_scoreboard(rows, out_dir / "scoreboard.md")


def _write_raw(rows: list[dict], path: Path) -> None:
    with path.open("w") as fh:
        for row in rows:
            fh.write(json.dumps(row) + "\n")


def _write_scoreboard(rows: list[dict], path: Path) -> None:
    overall: dict[str, Tally] = defaultdict(Tally)
    by_cat: dict[tuple[str, str], Tally] = defaultdict(Tally)
    categories: set[str] = set()
    for r in rows:
        actual = Label(r["actual"])
        overall[r["strategy"]].add(r["predicted_stale"], actual)
        by_cat[(r["strategy"], r["category"])].add(r["predicted_stale"], actual)
        categories.add(r["category"])

    lines = ["# Staleness Spike Scoreboard", ""]
    lines.append(f"Go bar: FP rate < {GO_BAR_FP:.0%} AND recall > {GO_BAR_RECALL:.0%}")
    lines.append("")
    lines.append("## Overall")
    lines.append("")
    lines.append("| Strategy | FP rate | Recall | Precision | F1 | Verdict |")
    lines.append("|---|---|---|---|---|---|")
    for strat in sorted(overall):
        m = metrics(overall[strat])
        verdict = "GO" if (m["fp_rate"] < GO_BAR_FP and m["recall"] > GO_BAR_RECALL) else "no-go"
        lines.append(
            f"| {strat} | {m['fp_rate']:.1%} | {m['recall']:.1%} | "
            f"{m['precision']:.1%} | {m['f1']:.2f} | {verdict} |"
        )

    lines.append("")
    lines.append("## False-positive rate by valid-category (lower is better)")
    lines.append("")
    valid_cats = sorted(c for c in categories if c in {"reformat", "rename-local", "move-symbol", "add-adjacent", "control"})
    header = "| Strategy | " + " | ".join(valid_cats) + " |"
    lines.append(header)
    lines.append("|" + "---|" * (len(valid_cats) + 1))
    for strat in sorted(overall):
        cells = []
        for cat in valid_cats:
            m = metrics(by_cat[(strat, cat)])
            cells.append(f"{m['fp_rate']:.0%}")
        lines.append(f"| {strat} | " + " | ".join(cells) + " |")

    path.write_text("\n".join(lines) + "\n")
