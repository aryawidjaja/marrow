from pathlib import Path

from staleness_spike.strategies.s2_normalized import S2Normalized
from staleness_spike.types import RawAnchor, SeededMemory


def _seed(tmp_repo, files, file_path, symbol, snippet):
    repo = tmp_repo(files)
    source = (repo / file_path).read_text()
    anchor = RawAnchor(id="m", file_path=file_path, symbol=symbol,
                       start_line=1, end_line=2, snippet=snippet)
    s = S2Normalized()
    mem = SeededMemory(anchor=anchor, payloads={s.name: s.seed(anchor, source)})
    return s, mem, repo


def test_s2_valid_after_whitespace_reformat(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("def f(x):\n\n        return x + 1\n")  # whitespace only
    assert s.check(mem, repo).is_stale is False


def test_s2_stale_when_token_text_changes(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    (repo / "a.py").write_text("def f(x):\n    return x + 2\n")
    assert s.check(mem, repo).is_stale is True


def test_s2_stale_when_block_only_in_another_file(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n", "b.py": "X = 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f", "def f(x):\n    return x + 1")
    # move the block to b.py, remove from a.py
    (repo / "a.py").write_text("X = 0\n")
    (repo / "b.py").write_text("X = 1\ndef f(x):\n    return x + 1\n")
    assert s.check(mem, repo).is_stale is True  # only searches the original file
