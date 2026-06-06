from __future__ import annotations

import ast
from pathlib import Path

from staleness_spike.astutils import find_symbol, node_segment
from staleness_spike.types import MutationCategory, RawAnchor


def apply_mutation(repo_root: Path, anchor: RawAnchor, category: MutationCategory) -> bool:
    """Apply one labeled edit in place. Return False if the target is unsuitable."""
    handler = _HANDLERS[category]
    return handler(repo_root, anchor)


def _read(repo_root: Path, rel: str) -> str:
    return (repo_root / rel).read_text(encoding="utf-8", errors="replace")


def _write(repo_root: Path, rel: str, text: str) -> None:
    (repo_root / rel).write_text(text, encoding="utf-8")


def _replace_segment(source: str, node: ast.AST, new_seg: str) -> str:
    """Replace the node's line span with new_seg, preserving the rest of the file."""
    lines = source.splitlines(keepends=True)
    before = lines[: node.lineno - 1]
    after = lines[node.end_lineno:]
    block = new_seg if new_seg.endswith("\n") else new_seg + "\n"
    return "".join(before) + block + "".join(after)


def _control(repo_root: Path, anchor: RawAnchor) -> bool:
    return True


def _reformat(repo_root: Path, anchor: RawAnchor) -> bool:
    # Whitespace-only: blank line after the def line + doubled leading indentation.
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    seg_lines = node_segment(src, node).splitlines()
    reflowed = [seg_lines[0], ""] + ["    " + ln for ln in seg_lines[1:]]
    _write(repo_root, anchor.file_path, _replace_segment(src, node, "\n".join(reflowed)))
    return True


class _LocalRenamer(ast.NodeTransformer):
    def __init__(self, old: str, new: str) -> None:
        self.old, self.new = old, new

    def visit_Name(self, node: ast.Name) -> ast.Name:
        if node.id == self.old:
            node.id = self.new
        return node


def _first_local(node: ast.AST) -> str | None:
    for sub in ast.walk(node):
        if isinstance(sub, ast.Assign):
            for tgt in sub.targets:
                if isinstance(tgt, ast.Name):
                    return tgt.id
    return None


def _rename_local(repo_root: Path, anchor: RawAnchor) -> bool:
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    local = _first_local(node)
    if local is None:
        return False  # unsuitable: no local to rename
    renamed = _LocalRenamer(local, local + "_renamed").visit(ast.parse(node_segment(src, node)))
    ast.fix_missing_locations(renamed)
    _write(repo_root, anchor.file_path, _replace_segment(src, node, ast.unparse(renamed)))
    return True


def _move_symbol(repo_root: Path, anchor: RawAnchor) -> bool:
    targets = [p for p in sorted(repo_root.rglob("*.py"))
               if p.relative_to(repo_root).as_posix() != anchor.file_path]
    if not targets:
        return False  # need a different file to move into
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    seg = node_segment(src, node)
    # Remove from origin.
    lines = src.splitlines(keepends=True)
    remaining = "".join(lines[: node.lineno - 1] + lines[node.end_lineno:])
    _write(repo_root, anchor.file_path, remaining if remaining.strip() else "X = 0\n")
    # Append to the first other file.
    dest = targets[0]
    dest.write_text(dest.read_text(encoding="utf-8", errors="replace").rstrip("\n") + "\n" + seg.rstrip("\n") + "\n", encoding="utf-8")
    return True


def _add_adjacent(repo_root: Path, anchor: RawAnchor) -> bool:
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    lines = src.splitlines(keepends=True)
    insert = "def _spike_unrelated():\n    return 0\n\n"
    patched = "".join(lines[: node.lineno - 1]) + insert + "".join(lines[node.lineno - 1:])
    _write(repo_root, anchor.file_path, patched)
    return True


def _change_logic(repo_root: Path, anchor: RawAnchor) -> bool:
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    tree = ast.parse(node_segment(src, node))
    funcs = [n for n in ast.walk(tree) if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))]
    if not funcs:
        return False  # unsuitable: class with no method body to alter
    side_effect = ast.parse("print('spike-mutation')").body[0]
    funcs[0].body.insert(0, side_effect)
    ast.fix_missing_locations(tree)
    _write(repo_root, anchor.file_path, _replace_segment(src, node, ast.unparse(tree)))
    return True


def _change_signature(repo_root: Path, anchor: RawAnchor) -> bool:
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    tree = ast.parse(node_segment(src, node))
    funcs = [n for n in ast.walk(tree) if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))]
    if not funcs:
        return False  # unsuitable: no signature to change
    funcs[0].args.args.append(ast.arg(arg="_spike_param"))
    ast.fix_missing_locations(tree)
    _write(repo_root, anchor.file_path, _replace_segment(src, node, ast.unparse(tree)))
    return True


def _delete_symbol(repo_root: Path, anchor: RawAnchor) -> bool:
    src = _read(repo_root, anchor.file_path)
    node = find_symbol(src, anchor.symbol)
    lines = src.splitlines(keepends=True)
    remaining = "".join(lines[: node.lineno - 1] + lines[node.end_lineno:])
    _write(repo_root, anchor.file_path, remaining if remaining.strip() else "X = 0\n")
    return True


_HANDLERS = {
    MutationCategory.CONTROL: _control,
    MutationCategory.REFORMAT: _reformat,
    MutationCategory.RENAME_LOCAL: _rename_local,
    MutationCategory.MOVE_SYMBOL: _move_symbol,
    MutationCategory.ADD_ADJACENT: _add_adjacent,
    MutationCategory.CHANGE_LOGIC: _change_logic,
    MutationCategory.CHANGE_SIGNATURE: _change_signature,
    MutationCategory.DELETE_SYMBOL: _delete_symbol,
}
