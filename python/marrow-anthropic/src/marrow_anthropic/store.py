"""Filesystem backend for Anthropic's memory tool, implementing the six commands.

All paths are confined to a base directory exposed to the model as ``/memories``. The
return strings follow the memory tool specification so the model sees familiar output.
"""

from __future__ import annotations

import shutil
from pathlib import Path

MEMORY_ROOT = "/memories"


class MemoryStore:
    """Stores memory files under ``base_dir``, addressed via ``/memories`` paths."""

    def __init__(self, base_dir: str | Path) -> None:
        self.base = Path(base_dir).resolve()
        self.base.mkdir(parents=True, exist_ok=True)

    # -- path handling -----------------------------------------------------

    def _resolve(self, tool_path: str) -> Path:
        """Map a ``/memories/...`` path to a real path, rejecting traversal."""
        if tool_path != MEMORY_ROOT and not tool_path.startswith(MEMORY_ROOT + "/"):
            raise _OutsideMemory(tool_path)
        rel = tool_path[len(MEMORY_ROOT):].lstrip("/")
        target = (self.base / rel).resolve()
        if target != self.base and self.base not in target.parents:
            raise _OutsideMemory(tool_path)
        return target

    # -- commands ----------------------------------------------------------

    def view(self, path: str, view_range: list[int] | None = None) -> str:
        try:
            target = self._resolve(path)
        except _OutsideMemory as e:
            return str(e)
        if not target.exists():
            return f"The path {path} does not exist. Please provide a valid path."
        if target.is_dir():
            return _list_dir(path, target)
        return _view_file(path, target, view_range)

    def create(self, path: str, file_text: str) -> str:
        try:
            target = self._resolve(path)
        except _OutsideMemory as e:
            return str(e)
        if target.exists():
            return f"Error: File {path} already exists"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(file_text, encoding="utf-8")
        return f"File created successfully at: {path}"

    def str_replace(self, path: str, old_str: str, new_str: str) -> str:
        try:
            target = self._resolve(path)
        except _OutsideMemory as e:
            return str(e)
        if not target.is_file():
            return f"Error: The path {path} does not exist. Please provide a valid path."
        content = target.read_text(encoding="utf-8")
        count = content.count(old_str)
        if count == 0:
            return f"No replacement was performed, old_str `{old_str}` did not appear verbatim in {path}."
        if count > 1:
            lines = _match_lines(content, old_str)
            return (
                f"No replacement was performed. Multiple occurrences of old_str `{old_str}` "
                f"in lines: {lines}. Please ensure it is unique"
            )
        target.write_text(content.replace(old_str, new_str), encoding="utf-8")
        return "The memory file has been edited."

    def insert(self, path: str, insert_line: int, insert_text: str) -> str:
        try:
            target = self._resolve(path)
        except _OutsideMemory as e:
            return str(e)
        if not target.is_file():
            return f"Error: The path {path} does not exist"
        lines = target.read_text(encoding="utf-8").splitlines(keepends=True)
        if insert_line < 0 or insert_line > len(lines):
            return (
                f"Error: Invalid `insert_line` parameter: {insert_line}. "
                f"It should be within the range of lines of the file: [0, {len(lines)}]"
            )
        snippet = insert_text if insert_text.endswith("\n") else insert_text + "\n"
        lines.insert(insert_line, snippet)
        target.write_text("".join(lines), encoding="utf-8")
        return f"The file {path} has been edited."

    def delete(self, path: str) -> str:
        try:
            target = self._resolve(path)
        except _OutsideMemory as e:
            return str(e)
        if not target.exists():
            return f"Error: The path {path} does not exist"
        if target.is_dir():
            shutil.rmtree(target)
        else:
            target.unlink()
        return f"Successfully deleted {path}"

    def rename(self, old_path: str, new_path: str) -> str:
        try:
            src = self._resolve(old_path)
            dst = self._resolve(new_path)
        except _OutsideMemory as e:
            return str(e)
        if not src.exists():
            return f"Error: The path {old_path} does not exist"
        if dst.exists():
            return f"Error: The destination {new_path} already exists"
        dst.parent.mkdir(parents=True, exist_ok=True)
        src.rename(dst)
        return f"Successfully renamed {old_path} to {new_path}"

    def clear(self) -> str:
        for child in self.base.iterdir():
            if child.is_dir():
                shutil.rmtree(child)
            else:
                child.unlink()
        return "Memory cleared."


class _OutsideMemory(Exception):
    def __init__(self, path: str) -> None:
        super().__init__(f"Error: The path {path} is outside the /memories directory")


def _human_size(num: int) -> str:
    size = float(num)
    for unit in ("", "K", "M", "G"):
        if size < 1024 or unit == "G":
            return f"{size:.1f}{unit}" if unit else f"{int(size)}"
        size /= 1024
    return f"{size:.1f}G"


def _list_dir(path: str, target: Path) -> str:
    entries: list[tuple[str, int]] = []
    for child in sorted(target.rglob("*")):
        rel = child.relative_to(target)
        if len(rel.parts) > 2 or any(p.startswith(".") or p == "node_modules" for p in rel.parts):
            continue
        size = child.stat().st_size if child.is_file() else 0
        entries.append((f"{path.rstrip('/')}/{rel.as_posix()}", size))
    header = (
        f"Here're the files and directories up to 2 levels deep in {path}, "
        "excluding hidden items and node_modules:"
    )
    total = sum(s for _, s in entries)
    lines = [header, f"{_human_size(total)}\t{path}"]
    lines += [f"{_human_size(size)}\t{name}" for name, size in entries]
    return "\n".join(lines)


def _view_file(path: str, target: Path, view_range: list[int] | None) -> str:
    lines = target.read_text(encoding="utf-8").splitlines()
    start, end = 1, len(lines)
    if view_range and len(view_range) == 2:
        start, end = view_range
    numbered = [
        f"{i:>6}\t{line}"
        for i, line in enumerate(lines[start - 1: end], start=start)
    ]
    return f"Here's the content of {path} with line numbers:\n" + "\n".join(numbered)


def _match_lines(content: str, needle: str) -> str:
    matches = [str(i) for i, line in enumerate(content.splitlines(), 1) if needle in line]
    return ", ".join(matches)
