from pathlib import Path

from staleness_spike.strategies.s4_relocation import S4Relocation
from staleness_spike.types import RawAnchor, SeededMemory


def _seed(tmp_repo, files, file_path, symbol, snippet):
    repo = tmp_repo(files)
    source = (repo / file_path).read_text()
    anchor = RawAnchor(id="m", file_path=file_path, symbol=symbol,
                       start_line=1, end_line=2, snippet=snippet)
    s = S4Relocation()
    mem = SeededMemory(anchor=anchor, payloads={s.name: s.seed(anchor, source)})
    return s, mem, repo


def test_s4_valid_and_relocates_after_cross_file_move(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n", "b.py": "Y = 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("Z = 1\n")
    (repo / "b.py").write_text("Y = 1\ndef f(x):\n    return x + 1\n")
    verdict = s.check(mem, repo)
    assert verdict.is_stale is False
    assert verdict.relocated_to is not None
    assert verdict.relocated_to.startswith("b.py:")


def test_s4_stale_when_content_changed(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("def f(x):\n    return x + 99\n")
    assert s.check(mem, repo).is_stale is True


def test_s4_stale_when_deleted_entirely(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("X = 1\n")
    assert s.check(mem, repo).is_stale is True


def test_s4_valid_in_place_when_unchanged(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    assert s.check(mem, repo).is_stale is False


def test_s4_valid_after_whitespace_reformat(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("def f(x):\n\n        return x + 1\n")  # whitespace only
    assert s.check(mem, repo).is_stale is False
