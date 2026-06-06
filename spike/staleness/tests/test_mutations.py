from pathlib import Path

from staleness_spike.astutils import find_symbol, fingerprint, node_segment, normalize_ws
from staleness_spike.mutations import apply_mutation
from staleness_spike.types import MutationCategory, RawAnchor


def _anchor(repo: Path, rel: str, symbol: str) -> RawAnchor:
    source = (repo / rel).read_text()
    node = find_symbol(source, symbol)
    seg = node_segment(source, node)
    return RawAnchor(id="m", file_path=rel, symbol=symbol,
                     start_line=node.lineno, end_line=node.end_lineno, snippet=seg)


def _write(tmp_path, rel, src):
    p = tmp_path / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(src)
    return tmp_path


FUNC = "def f(x):\n    total = x + 1\n    return total\n"


def test_control_changes_nothing(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    before = (repo / "a.py").read_text()
    applied = apply_mutation(repo, _anchor(repo, "a.py", "f"), MutationCategory.CONTROL)
    assert applied is True
    assert (repo / "a.py").read_text() == before


def test_reformat_preserves_normalized_text(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    apply_mutation(repo, anchor, MutationCategory.REFORMAT)
    new_seg = node_segment((repo / "a.py").read_text(), find_symbol((repo / "a.py").read_text(), "f"))
    assert normalize_ws(new_seg) == normalize_ws(anchor.snippet)


def test_rename_local_preserves_fingerprint_changes_text(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    assert apply_mutation(repo, anchor, MutationCategory.RENAME_LOCAL) is True
    src = (repo / "a.py").read_text()
    new_seg = node_segment(src, find_symbol(src, "f"))
    assert fingerprint(new_seg) == fingerprint(anchor.snippet)  # behavior preserved
    assert normalize_ws(new_seg) != normalize_ws(anchor.snippet)  # text changed


def test_move_symbol_removes_from_origin_adds_elsewhere(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    (repo / "b.py").write_text("Y = 1\n")
    anchor = _anchor(repo, "a.py", "f")
    assert apply_mutation(repo, anchor, MutationCategory.MOVE_SYMBOL) is True
    assert find_symbol((repo / "a.py").read_text(), "f") is None
    assert find_symbol((repo / "b.py").read_text(), "f") is not None


def test_add_adjacent_keeps_symbol_intact(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    apply_mutation(repo, anchor, MutationCategory.ADD_ADJACENT)
    src = (repo / "a.py").read_text()
    assert find_symbol(src, "f") is not None
    assert "_spike_unrelated" in src


def test_change_logic_changes_fingerprint(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    apply_mutation(repo, anchor, MutationCategory.CHANGE_LOGIC)
    src = (repo / "a.py").read_text()
    assert fingerprint(node_segment(src, find_symbol(src, "f"))) != fingerprint(anchor.snippet)


def test_change_signature_adds_param(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    apply_mutation(repo, anchor, MutationCategory.CHANGE_SIGNATURE)
    src = (repo / "a.py").read_text()
    node = find_symbol(src, "f")
    assert len(node.args.args) == 2


def test_delete_symbol_removes_it(tmp_path):
    repo = _write(tmp_path, "a.py", FUNC)
    anchor = _anchor(repo, "a.py", "f")
    apply_mutation(repo, anchor, MutationCategory.DELETE_SYMBOL)
    assert find_symbol((repo / "a.py").read_text(), "f") is None
