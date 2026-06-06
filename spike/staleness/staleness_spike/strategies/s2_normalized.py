from __future__ import annotations

import hashlib
from pathlib import Path

from staleness_spike.astutils import normalize_ws
from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S2Normalized:
    """The MARROW.md proposal: normalized-whitespace hash, searched within the file."""

    name = "s2_normalized"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        return {"norm_hash": _sha(normalize_ws(anchor.snippet))}

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        anchor = memory.anchor
        path = repo_root / anchor.file_path
        target = memory.payloads[self.name]["norm_hash"]
        if not path.exists():
            return Verdict(is_stale=True)
        if _contains_block(path.read_text(encoding="utf-8", errors="replace"), anchor.snippet, target):
            return Verdict(is_stale=False)
        return Verdict(is_stale=True)


def _contains_block(haystack: str, needle: str, target_hash: str) -> bool:
    """True if a contiguous line-window of haystack matches the needle's norm hash.

    Tries every possible window size so that blank-line insertions (whitespace
    reformatting) are still detected as matching.
    """
    hay_lines = haystack.splitlines()
    min_window = max(1, len(needle.splitlines()))
    for window in range(min_window, len(hay_lines) + 1):
        for i in range(0, len(hay_lines) - window + 1):
            chunk = "\n".join(hay_lines[i: i + window])
            if _sha(normalize_ws(chunk)) == target_hash:
                return True
    return False


def _sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
