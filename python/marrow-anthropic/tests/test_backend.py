import pytest

pytest.importorskip("anthropic")

from marrow_anthropic import MarrowMemoryBackend


def test_to_dict_declares_the_memory_tool():
    backend = MarrowMemoryBackend("/tmp/marrow-mem-unused")
    assert backend.to_dict()["type"] == "memory_20250818"


def test_full_command_lifecycle_through_sdk_dispatch(tmp_path):
    # `call` constructs the SDK's typed command from a dict and dispatches via execute().
    backend = MarrowMemoryBackend(tmp_path)

    assert "created successfully" in backend.call(
        {"command": "create", "path": "/memories/n.txt", "file_text": "hi\n"}
    )
    assert "with line numbers" in backend.call({"command": "view", "path": "/memories/n.txt"})
    assert "edited" in backend.call(
        {"command": "str_replace", "path": "/memories/n.txt", "old_str": "hi", "new_str": "yo"}
    )
    assert "edited" in backend.call(
        {"command": "insert", "path": "/memories/n.txt", "insert_line": 0, "insert_text": "top\n"}
    )
    assert "renamed" in backend.call(
        {"command": "rename", "old_path": "/memories/n.txt", "new_path": "/memories/m.txt"}
    )
    assert "deleted" in backend.call({"command": "delete", "path": "/memories/m.txt"})
