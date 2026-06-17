"""Drop-in backend for Anthropic's memory tool, backed by :class:`MemoryStore`.

Requires the ``anthropic`` SDK (install ``marrow-anthropic[sdk]``). Usage::

    from anthropic import Anthropic
    from marrow_anthropic import MarrowMemoryBackend

    client = Anthropic()
    memory = MarrowMemoryBackend("./.marrow/memories")
    client.beta.messages.run_tools(model="claude-opus-4-8", messages=[...], tools=[memory])
"""

from __future__ import annotations

from pathlib import Path

from anthropic.tools import BetaAbstractMemoryTool

from .store import MemoryStore


class MarrowMemoryBackend(BetaAbstractMemoryTool):
    """Maps memory tool commands onto a :class:`MemoryStore`."""

    def __init__(self, base_dir: str | Path = "./.marrow/memories", **kwargs: object) -> None:
        super().__init__(**kwargs)  # type: ignore[arg-type]
        self.store = MemoryStore(base_dir)

    def view(self, command):  # type: ignore[override]
        return self.store.view(command.path, getattr(command, "view_range", None))

    def create(self, command):  # type: ignore[override]
        return self.store.create(command.path, command.file_text)

    def str_replace(self, command):  # type: ignore[override]
        return self.store.str_replace(command.path, command.old_str, command.new_str)

    def insert(self, command):  # type: ignore[override]
        return self.store.insert(command.path, command.insert_line, command.insert_text)

    def delete(self, command):  # type: ignore[override]
        return self.store.delete(command.path)

    def rename(self, command):  # type: ignore[override]
        return self.store.rename(command.old_path, command.new_path)

    def clear_all_memory(self):  # type: ignore[override]
        return self.store.clear()
