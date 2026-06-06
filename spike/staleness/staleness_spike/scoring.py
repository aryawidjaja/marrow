from __future__ import annotations

from dataclasses import dataclass

from staleness_spike.types import Label


@dataclass
class Tally:
    tp: int = 0  # predicted stale, actually stale
    fp: int = 0  # predicted stale, actually valid  <- the trust-killer
    tn: int = 0  # predicted valid, actually valid
    fn: int = 0  # predicted valid, actually stale

    def add(self, predicted_stale: bool, actual: Label) -> None:
        if actual == Label.STALE:
            self.tp += int(predicted_stale)
            self.fn += int(not predicted_stale)
        else:
            self.fp += int(predicted_stale)
            self.tn += int(not predicted_stale)


def metrics(t: Tally) -> dict[str, float]:
    actual_valid = t.fp + t.tn
    actual_stale = t.tp + t.fn
    predicted_stale = t.tp + t.fp
    fp_rate = t.fp / actual_valid if actual_valid else 0.0
    recall = t.tp / actual_stale if actual_stale else 0.0
    precision = t.tp / predicted_stale if predicted_stale else 0.0
    f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) else 0.0
    return {"fp_rate": fp_rate, "recall": recall, "precision": precision, "f1": f1}
