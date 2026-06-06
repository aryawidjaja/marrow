from pathlib import Path

from staleness_spike.seeding import seed_anchors


def test_seed_anchors_is_deterministic(tmp_repo):
    files = {
        "pkg/a.py": "def f(x):\n    return x\n\ndef g(y):\n    return y\n",
        "pkg/b.py": "class C:\n    def m(self):\n        return 1\n",
    }
    repo = tmp_repo(files)
    a = seed_anchors(repo, count=3, seed=42)
    b = seed_anchors(repo, count=3, seed=42)
    assert [x.symbol for x in a] == [x.symbol for x in b]


def test_seed_anchors_respects_count_and_extracts_snippet(tmp_repo):
    files = {"pkg/a.py": "def f(x):\n    return x\n\ndef g(y):\n    return y\n"}
    repo = tmp_repo(files)
    anchors = seed_anchors(repo, count=2, seed=1)
    assert len(anchors) == 2
    for a in anchors:
        assert a.snippet.startswith("def ")
        assert a.file_path == "pkg/a.py"
        assert a.start_line >= 1 and a.end_line >= a.start_line


def test_seed_anchors_caps_at_available_symbols(tmp_repo):
    files = {"pkg/a.py": "def only():\n    return 1\n"}
    repo = tmp_repo(files)
    anchors = seed_anchors(repo, count=50, seed=1)
    assert len(anchors) == 1


def test_seed_anchors_skips_nested_methods(tmp_repo):
    files = {"pkg/a.py": "class C:\n    def m(self):\n        return 1\n\ndef top():\n    return 2\n"}
    repo = tmp_repo(files)
    anchors = seed_anchors(repo, count=10, seed=1)
    symbols = {a.symbol for a in anchors}
    assert "top" in symbols
    assert "C" in symbols
    assert "C.m" not in symbols  # nested methods are excluded (col_offset != 0)
