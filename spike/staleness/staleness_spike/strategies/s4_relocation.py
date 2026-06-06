from __future__ import annotations

import hashlib
from pathlib import Path

from staleness_spike.astutils import normalize_ws
from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S4Relocation:
    """Normalized-hash content match searched across the WHOLE repo, with relocation."""

    name = "s4_relocation"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        return {
            "norm_hash": _sha(normalize_ws(anchor.snippet)),
            "min_window": max(1, len(anchor.snippet.splitlines())),
        }

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        payload = memory.payloads[self.name]
        target = payload["norm_hash"]
        min_window = payload["min_window"]
        origin = memory.anchor.file_path
        for path in sorted(repo_root.rglob("*.py")):
            hit = _find_window(path.read_text(encoding="utf-8", errors="replace"), min_window, target)
            if hit is not None:
                rel = path.relative_to(repo_root).as_posix()
                relocated = None if rel == origin else f"{rel}:{hit}"
                return Verdict(is_stale=False, relocated_to=relocated)
        return Verdict(is_stale=True)


def _find_window(haystack: str, min_window: int, target_hash: str) -> int | None:
    """Return 1-based start line of the first matching window, else None.

    Tries every window size >= min_window so whitespace reformatting (e.g. an
    inserted blank line) is still matched.
    """
    lines = haystack.splitlines()
    for window in range(min_window, len(lines) + 1):
        for i in range(0, len(lines) - window + 1):
            chunk = "\n".join(lines[i: i + window])
            if _sha(normalize_ws(chunk)) == target_hash:
                return i + 1
    return None


def _sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()
