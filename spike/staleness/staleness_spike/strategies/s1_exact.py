from __future__ import annotations

import hashlib
from pathlib import Path

from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S1Exact:
    """Floor baseline: SHA-256 of the raw snippet at the exact recorded line range."""

    name = "s1_exact"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        return {"hash": _sha(anchor.snippet)}

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        anchor = memory.anchor
        path = repo_root / anchor.file_path
        if not path.exists():
            return Verdict(is_stale=True)
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        current = "\n".join(lines[anchor.start_line - 1: anchor.end_line])
        stale = _sha(current) != memory.payloads[self.name]["hash"]
        return Verdict(is_stale=stale)


def _sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
