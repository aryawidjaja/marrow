from __future__ import annotations

import ast
import hashlib
import re
from typing import Iterator

_DEF_TYPES = (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)


def parse(source: str) -> ast.Module:
    return ast.parse(source)


def iter_symbols(source: str) -> Iterator[tuple[str, ast.AST]]:
    """Yield (qualified_name, node) for every def/class, nesting via dotted names."""
    module = parse(source)

    def walk(node: ast.AST, prefix: str) -> Iterator[tuple[str, ast.AST]]:
        for child in ast.iter_child_nodes(node):
            if isinstance(child, _DEF_TYPES):
                qn = f"{prefix}{child.name}"
                yield qn, child
                yield from walk(child, f"{qn}.")

    yield from walk(module, "")


def find_symbol(source: str, qualified_name: str) -> ast.AST | None:
    for qn, node in iter_symbols(source):
        if qn == qualified_name:
            return node
    return None


def node_segment(source: str, node: ast.AST) -> str:
    seg = ast.get_source_segment(source, node)
    return seg if seg is not None else ""


def normalize_ws(text: str) -> str:
    """Collapse all runs of whitespace to single spaces and strip ends."""
    return re.sub(r"\s+", " ", text).strip()


class _AlphaRenamer(ast.NodeTransformer):
    """Rename all Name nodes and args to canonical __v0, __v1, ... positionally."""

    def __init__(self) -> None:
        self._mapping: dict[str, str] = {}

    def _canonical(self, name: str) -> str:
        if name not in self._mapping:
            self._mapping[name] = f"__v{len(self._mapping)}"
        return self._mapping[name]

    def visit_arg(self, node: ast.arg) -> ast.arg:
        node.arg = self._canonical(node.arg)
        return node

    def visit_Name(self, node: ast.Name) -> ast.Name:
        node.id = self._canonical(node.id)
        return node


def fingerprint(segment: str) -> str:
    """Structural hash: parse, drop docstrings, alpha-rename all names, canonical unparse.

    Stable across reformatting and local-variable renames; changes when control
    flow, operators, or argument count change.

    NOTE: every Name node (including builtins and globals) is alpha-renamed, so
    `return len(x)` and `return str(x)` hash identically. Changes to which
    non-local name is referenced are NOT detected. Accepted simplification for
    the spike.
    """
    tree = ast.parse(segment)  # fresh tree each call; _AlphaRenamer mutates in place
    # Drop module/function/class docstrings (first string-expr statements).
    for node in ast.walk(tree):
        body = getattr(node, "body", None)
        if isinstance(body, list) and body:
            first = body[0]
            if (
                isinstance(first, ast.Expr)
                and isinstance(first.value, ast.Constant)
                and isinstance(first.value.value, str)
            ):
                body.pop(0)
    renamed = _AlphaRenamer().visit(tree)
    ast.fix_missing_locations(renamed)
    canonical = ast.unparse(renamed)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()
