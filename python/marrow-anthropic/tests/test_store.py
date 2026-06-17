from marrow_anthropic import MemoryStore


def store(tmp_path):
    return MemoryStore(tmp_path)


def test_create_and_view_file(tmp_path):
    s = store(tmp_path)
    assert s.create("/memories/notes.txt", "hello\nworld\n") == (
        "File created successfully at: /memories/notes.txt"
    )
    out = s.view("/memories/notes.txt")
    assert "with line numbers" in out
    assert "\thello" in out
    assert "     1\t" in out  # 6-wide, 1-indexed


def test_create_rejects_existing(tmp_path):
    s = store(tmp_path)
    s.create("/memories/a.txt", "x")
    assert s.create("/memories/a.txt", "y") == "Error: File /memories/a.txt already exists"


def test_view_directory_lists_entries(tmp_path):
    s = store(tmp_path)
    s.create("/memories/a.txt", "aaa")
    s.create("/memories/sub/b.txt", "bbb")
    out = s.view("/memories")
    assert "/memories/a.txt" in out
    assert "/memories/sub/b.txt" in out


def test_view_missing_path(tmp_path):
    s = store(tmp_path)
    assert s.view("/memories/nope.txt") == (
        "The path /memories/nope.txt does not exist. Please provide a valid path."
    )


def test_view_range(tmp_path):
    s = store(tmp_path)
    s.create("/memories/f.txt", "l1\nl2\nl3\nl4\n")
    out = s.view("/memories/f.txt", [2, 3])
    assert "l2" in out and "l3" in out
    assert "l1" not in out and "l4" not in out


def test_str_replace_unique(tmp_path):
    s = store(tmp_path)
    s.create("/memories/p.txt", "color: blue\n")
    assert s.str_replace("/memories/p.txt", "blue", "green") == "The memory file has been edited."
    assert "green" in s.view("/memories/p.txt")


def test_str_replace_not_found(tmp_path):
    s = store(tmp_path)
    s.create("/memories/p.txt", "color: blue\n")
    msg = s.str_replace("/memories/p.txt", "red", "green")
    assert "did not appear verbatim" in msg


def test_str_replace_multiple_occurrences(tmp_path):
    s = store(tmp_path)
    s.create("/memories/p.txt", "x\nx\n")
    msg = s.str_replace("/memories/p.txt", "x", "y")
    assert "Multiple occurrences" in msg
    assert "1, 2" in msg


def test_insert(tmp_path):
    s = store(tmp_path)
    s.create("/memories/t.txt", "a\nc\n")
    assert s.insert("/memories/t.txt", 1, "b") == "The file /memories/t.txt has been edited."
    assert s.view("/memories/t.txt").count("\t") >= 3
    body = "".join(s.view("/memories/t.txt").split("with line numbers:\n")[1])
    assert "a" in body and "b" in body and "c" in body


def test_insert_invalid_line(tmp_path):
    s = store(tmp_path)
    s.create("/memories/t.txt", "a\n")
    assert "Invalid `insert_line`" in s.insert("/memories/t.txt", 99, "x")


def test_delete_file_and_dir(tmp_path):
    s = store(tmp_path)
    s.create("/memories/d/x.txt", "x")
    assert s.delete("/memories/d/x.txt") == "Successfully deleted /memories/d/x.txt"
    assert s.delete("/memories/d") == "Successfully deleted /memories/d"
    assert s.delete("/memories/gone") == "Error: The path /memories/gone does not exist"


def test_rename(tmp_path):
    s = store(tmp_path)
    s.create("/memories/draft.txt", "x")
    assert s.rename("/memories/draft.txt", "/memories/final.txt") == (
        "Successfully renamed /memories/draft.txt to /memories/final.txt"
    )
    assert s.view("/memories/final.txt").endswith("\tx")


def test_rename_destination_exists(tmp_path):
    s = store(tmp_path)
    s.create("/memories/a.txt", "1")
    s.create("/memories/b.txt", "2")
    assert s.rename("/memories/a.txt", "/memories/b.txt") == (
        "Error: The destination /memories/b.txt already exists"
    )


def test_path_traversal_is_blocked(tmp_path):
    s = store(tmp_path)
    for op in [
        s.view("/memories/../secret"),
        s.create("/memories/../escape.txt", "x"),
        s.view("/etc/passwd"),
    ]:
        assert "outside the /memories directory" in op


def test_clear(tmp_path):
    s = store(tmp_path)
    s.create("/memories/a.txt", "x")
    s.create("/memories/sub/b.txt", "y")
    s.clear()
    out = s.view("/memories")
    assert "a.txt" not in out and "b.txt" not in out
