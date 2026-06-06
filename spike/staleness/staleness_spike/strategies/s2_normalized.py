from __future__ import annotations

from pathlib import Path

from staleness_spike.astutils import normalize_ws
from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S2Normalized:
    """The MARROW.md proposal: whitespace-normalized snippet matched within the file."""

    name = "s2_normalized"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        return {"norm": normalize_ws(anchor.snippet)}

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        anchor = memory.anchor
        path = repo_root / anchor.file_path
        needle = memory.payloads[self.name]["norm"]
        if not path.exists():
            return Verdict(is_stale=True)
        text = path.read_text(encoding="utf-8", errors="replace")
        return Verdict(is_stale=needle not in normalize_ws(text))
