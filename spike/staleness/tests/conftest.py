import textwrap
from pathlib import Path

import pytest


@pytest.fixture
def tmp_repo(tmp_path: Path):
    """Write a small fake repo from a {relpath: source} mapping; return its root."""

    def _make(files: dict[str, str]) -> Path:
        for relpath, source in files.items():
            target = tmp_path / relpath
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(textwrap.dedent(source).lstrip("\n"))
        return tmp_path

    return _make
