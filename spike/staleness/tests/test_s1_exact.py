import hashlib
from pathlib import Path

from staleness_spike.strategies.s1_exact import S1Exact
from staleness_spike.types import RawAnchor, SeededMemory


def _seed(tmp_repo, files, file_path, symbol):
    repo = tmp_repo(files)
    source = (repo / file_path).read_text()
    # naive line span of the symbol for the anchor
    lines = source.splitlines()
    start = next(i for i, ln in enumerate(lines) if symbol.split(".")[-1] in ln and "def" in ln) + 1
    snippet = "\n".join(lines[start - 1:])
    anchor = RawAnchor(id="m", file_path=file_path, symbol=symbol,
                       start_line=start, end_line=len(lines), snippet=snippet)
    s = S1Exact()
    mem = SeededMemory(anchor=anchor, payloads={s.name: s.seed(anchor, source)})
    return s, mem, repo


def test_s1_valid_when_bytes_unchanged(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    assert s.check(mem, repo).is_stale is False


def test_s1_stale_when_any_byte_changes(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").write_text("def f(x):\n    return  x + 1\n")  # extra space only
    assert s.check(mem, repo).is_stale is True


def test_s1_stale_when_file_missing(tmp_repo):
    files = {"a.py": "def f(x):\n    return x + 1\n"}
    s, mem, repo = _seed(tmp_repo, files, "a.py", "f")
    (repo / "a.py").unlink()
    assert s.check(mem, repo).is_stale is True
