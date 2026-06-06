from __future__ import annotations

from pathlib import Path

from staleness_spike.astutils import normalize_ws
from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S4Relocation:
    """Whitespace-normalized snippet matched across the WHOLE repo, with relocation."""

    name = "s4_relocation"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        return {"norm": normalize_ws(anchor.snippet)}

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        anchor = memory.anchor
        needle = memory.payloads[self.name]["norm"]
        origin = anchor.file_path
        for path in sorted(repo_root.rglob("*.py")):
            text = path.read_text(encoding="utf-8", errors="replace")
            if needle in normalize_ws(text):
                rel = path.relative_to(repo_root).as_posix()
                relocated = None if rel == origin else f"{rel}:{_approx_line(text, anchor.snippet)}"
                return Verdict(is_stale=False, relocated_to=relocated)
        return Verdict(is_stale=True)


def _approx_line(text: str, snippet: str) -> int:
    """Best-effort 1-based line of the relocated match (for reporting only)."""
    first = next((ln.strip() for ln in snippet.splitlines() if ln.strip()), "")
    if first:
        for i, ln in enumerate(text.splitlines(), 1):
            if first in ln:
                return i
    return 1
