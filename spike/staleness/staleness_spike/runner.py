from __future__ import annotations

import shutil
import tempfile
from pathlib import Path

# Import strategy modules so their @register decorators populate the registry.
from staleness_spike.strategies import STRATEGIES
from staleness_spike.strategies import s1_exact, s2_normalized, s3_ast, s4_relocation  # noqa: F401
from staleness_spike.mutations import apply_mutation
from staleness_spike.seeding import seed_anchors
from staleness_spike.types import MutationCategory, SeededMemory, ground_truth


def _seed_memories(repo_root: Path, count: int, seed: int) -> list[SeededMemory]:
    anchors = seed_anchors(repo_root, count=count, seed=seed)
    memories: list[SeededMemory] = []
    for anchor in anchors:
        source = (repo_root / anchor.file_path).read_text(encoding="utf-8", errors="replace")
        payloads = {s.name: s.seed(anchor, source) for s in STRATEGIES}
        memories.append(SeededMemory(anchor=anchor, payloads=payloads))
    return memories


def run_spike(repo_root: Path, count: int, seed: int) -> list[dict]:
    """Return one result row per (memory, category, strategy) that applied."""
    memories = _seed_memories(repo_root, count=count, seed=seed)
    rows: list[dict] = []
    for memory in memories:
        for category in MutationCategory:
            with tempfile.TemporaryDirectory() as tmp:
                work = Path(tmp) / "repo"
                shutil.copytree(repo_root, work)
                applied = apply_mutation(work, memory.anchor, category)
                if not applied:
                    continue
                actual = ground_truth(category)
                for strategy in STRATEGIES:
                    verdict = strategy.check(memory, work)
                    rows.append({
                        "memory_id": memory.anchor.id,
                        "category": category.value,
                        "strategy": strategy.name,
                        "predicted_stale": verdict.is_stale,
                        "actual": actual.value,
                        "relocated_to": verdict.relocated_to,
                    })
    return rows
