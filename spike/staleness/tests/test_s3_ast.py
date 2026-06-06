from pathlib import Path

from staleness_spike.strategies.s3_ast import S3Ast
from staleness_spike.types import RawAnchor, SeededMemory


def _seed(tmp_repo, files, file_path, symbol):
    repo = tmp_repo(files)
    source = (repo / file_path).read_text()
    anchor = RawAnchor(id="m", file_path=file_path, symbol=symbol,
                       start_line=1, end_line=2, snippet="")
    s = S3Ast()
    mem = SeededMemory(anchor=anchor, payloads={s.name: s.seed(anchor, source)})
    return s, mem, repo


def test_s3_valid_after_reformat(tmp_repo):
    files = {"a.py": "def f(x):\n    total = x + 1\n    return total\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("def f(x):\n\n        total=x+1\n        return total\n")
    assert s.check(mem, repo).is_stale is False


def test_s3_valid_after_local_rename(tmp_repo):
    files = {"a.py": "def f(x):\n    total = x + 1\n    return total\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("def f(x):\n    result = x + 1\n    return result\n")
    assert s.check(mem, repo).is_stale is False


def test_s3_stale_after_logic_change(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("def f(x):\n    return x + 2\n")
    assert s.check(mem, repo).is_stale is True


def test_s3_stale_after_signature_change(tmp_repo):
    files = {"a.py": "def f(x):\n    return x\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("def f(x, y):\n    return x\n")
    assert s.check(mem, repo).is_stale is True


def test_s3_stale_when_symbol_deleted(tmp_repo):
    files = {"a.py": "def f(x):\n    return x\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("X = 1\n")
    assert s.check(mem, repo).is_stale is True


def test_s3_stale_when_moved_to_another_file(tmp_repo):
    files = {"a.py": "def f(x):\n    return x\n", "b.py": "Y = 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("Z = 1\n")
    (repo / "b.py").write_text("Y = 1\ndef f(x):\n    return x\n")
    # S3 only inspects the original file path, so a cross-file move reads as stale.
    assert s.check(mem, repo).is_stale is True
