from __future__ import annotations

import random
from pathlib import Path

from staleness_spike.astutils import iter_symbols, node_segment
from staleness_spike.types import RawAnchor


def seed_anchors(repo_root: Path, count: int, seed: int) -> list[RawAnchor]:
    """Discover top-level def/class symbols, deterministically sample `count` of them.

    Nested symbols (col_offset != 0, e.g. methods) are skipped: the mutation engine
    assumes column-0 anchors. Top-level classes still cover their method bodies.
    """
    candidates: list[RawAnchor] = []
    for path in sorted(repo_root.rglob("*.py")):
        rel = path.relative_to(repo_root).as_posix()
        try:
            source = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        try:
            symbols = list(iter_symbols(source))
        except SyntaxError:
            continue
        for qn, node in symbols:
            if getattr(node, "col_offset", 0) != 0:
                continue  # skip nested symbols; mutations assume top-level
            seg = node_segment(source, node)
            if not seg:
                continue
            candidates.append(
                RawAnchor(
                    id=f"{rel}::{qn}",
                    file_path=rel,
                    symbol=qn,
                    start_line=node.lineno,
                    end_line=node.end_lineno,
                    snippet=seg,
                )
            )
    candidates.sort(key=lambda a: a.id)  # stable order before sampling
    rng = random.Random(seed)
    if count >= len(candidates):
        return candidates
    return rng.sample(candidates, count)
