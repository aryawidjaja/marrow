from staleness_spike.astutils import (
    find_symbol,
    iter_symbols,
    node_segment,
    normalize_ws,
    fingerprint,
)

SRC = (
    "def outer(x):\n"
    "    total = x + 1\n"
    "    return total\n"
    "\n"
    "class C:\n"
    "    def m(self, y):\n"
    "        return y\n"
)


def test_iter_symbols_yields_qualified_names():
    names = {qn for qn, _ in iter_symbols(SRC)}
    assert names == {"outer", "C", "C.m"}


def test_find_symbol_returns_node_with_lines():
    node = find_symbol(SRC, "C.m")
    assert node is not None
    assert node.lineno == 6


def test_find_symbol_missing_returns_none():
    assert find_symbol(SRC, "nope") is None


def test_node_segment_extracts_source():
    node = find_symbol(SRC, "outer")
    seg = node_segment(SRC, node)
    assert seg.startswith("def outer(x):")
    assert "return total" in seg


def test_normalize_ws_collapses_whitespace():
    assert normalize_ws("def  f( ):\n    return   1") == "def f( ): return 1"


def test_fingerprint_ignores_formatting_and_local_names():
    a = "def f(x):\n    total = x + 1\n    return total"
    b = "def f(x):\n\n    result   =   x + 1\n    return result  # note"
    assert fingerprint(a) == fingerprint(b)


def test_fingerprint_changes_on_logic_change():
    a = "def f(x):\n    return x + 1"
    b = "def f(x):\n    return x + 2"
    assert fingerprint(a) != fingerprint(b)


def test_fingerprint_changes_on_signature_change():
    a = "def f(x):\n    return x"
    b = "def f(x, y):\n    return x"
    assert fingerprint(a) != fingerprint(b)


def test_node_segment_isolates_nested_method():
    node = find_symbol(SRC, "C.m")
    seg = node_segment(SRC, node)
    assert seg.startswith("def m(self, y):")
    assert "class C" not in seg
    assert "outer" not in seg
