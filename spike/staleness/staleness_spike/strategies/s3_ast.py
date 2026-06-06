from __future__ import annotations

from pathlib import Path

from staleness_spike.astutils import find_symbol, fingerprint, node_segment
from staleness_spike.strategies import register
from staleness_spike.types import RawAnchor, SeededMemory, Verdict


@register
class S3Ast:
    """Structural identity: normalized-AST fingerprint of the symbol in its file."""

    name = "s3_ast"

    def seed(self, anchor: RawAnchor, source: str) -> dict:
        node = find_symbol(source, anchor.symbol)
        seg = node_segment(source, node) if node is not None else ""
        return {"fingerprint": fingerprint(seg) if seg else None}

    def check(self, memory: SeededMemory, repo_root: Path) -> Verdict:
        anchor = memory.anchor
        path = repo_root / anchor.file_path
        target = memory.payloads[self.name]["fingerprint"]
        if not path.exists():
            return Verdict(is_stale=True)
        source = path.read_text()
        node = find_symbol(source, anchor.symbol)
        if node is None:
            return Verdict(is_stale=True)
        try:
            current_fp = fingerprint(node_segment(source, node))
        except SyntaxError:
            return Verdict(is_stale=True)
        return Verdict(is_stale=current_fp != target)
