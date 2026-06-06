from __future__ import annotations

from pathlib import Path
from typing import Protocol

from staleness_spike.types import RawAnchor, SeededMemory, Verdict


class Strategy(Protocol):
    name: str

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        """Compute the anchor payload from the ORIGINAL repo (read-only)."""
        ...

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        """Decide stale/valid against the (possibly mutated) repo."""
        ...
