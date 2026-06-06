from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


class Label(str, Enum):
    STALE = "stale"
    VALID = "valid"


class MutationCategory(str, Enum):
    REFORMAT = "reformat"
    RENAME_LOCAL = "rename-local"
    MOVE_SYMBOL = "move-symbol"
    ADD_ADJACENT = "add-adjacent"
    CHANGE_LOGIC = "change-logic"
    CHANGE_SIGNATURE = "change-signature"
    DELETE_SYMBOL = "delete-symbol"
    CONTROL = "control"


_STALE_CATEGORIES = {
    MutationCategory.CHANGE_LOGIC,
    MutationCategory.CHANGE_SIGNATURE,
    MutationCategory.DELETE_SYMBOL,
}


def ground_truth(category: MutationCategory) -> Label:
    """A mutation is STALE iff it changes the symbol's behavior or identity."""
    return Label.STALE if category in _STALE_CATEGORIES else Label.VALID


@dataclass(frozen=True)
class Verdict:
    is_stale: bool
    relocated_to: str | None = None  # "path:line" when a strategy relocates the citation


@dataclass
class RawAnchor:
    id: str
    file_path: str   # relative to repo root
    symbol: str      # qualified name, e.g. "module.Class.method"
    start_line: int  # 1-based, inclusive
    end_line: int    # 1-based, inclusive
    snippet: str     # raw source text of the symbol


@dataclass
class SeededMemory:
    anchor: RawAnchor
    payloads: dict[str, dict] = field(default_factory=dict)  # strategy name -> anchor payload
