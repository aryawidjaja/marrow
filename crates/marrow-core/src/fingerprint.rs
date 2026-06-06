//! Structural AST fingerprint (the S3 check).
//!
//! Walks a symbol's tree-sitter subtree and builds a canonical string that is stable
//! across reformatting and identifier renames, but changes when the structure, operators,
//! literal values, or signature change. Comments (including doc comments) are dropped.
//!
//! Every identifier is canonicalized to a positional token (`#0`, `#1`, ...). This is an
//! accepted simplification: the fingerprint is blind to *which* non-local name is
//! referenced (e.g. `foo()` vs `bar()` hash the same), but it never false-flags a rename.

use std::collections::HashMap;

use sha2::{Digest, Sha256};
use tree_sitter::Node;

use crate::parser::parse;
use crate::symbols::find_symbol;

/// Comment node kinds, skipped entirely (subtree included).
const COMMENT_KINDS: &[&str] = &["line_comment", "block_comment"];

/// Identifier node kinds, canonicalized to positional tokens.
const IDENT_KINDS: &[&str] = &[
    "identifier",
    "type_identifier",
    "field_identifier",
    "shorthand_field_identifier",
];

/// Structural fingerprint of `symbol` in `source`, hex-encoded. `None` if the source
/// can't be parsed or the symbol isn't found.
pub fn fingerprint(source: &str, symbol: &str) -> Option<String> {
    let (start, end) = find_symbol(source, symbol)?;
    let tree = parse(source)?;
    let node = node_at_range(tree.root_node(), start, end)?;
    let mut idents: HashMap<String, usize> = HashMap::new();
    let mut tokens = String::new();
    serialize(node, source, &mut idents, &mut tokens);
    let mut hasher = Sha256::new();
    hasher.update(tokens.as_bytes());
    Some(hex(&hasher.finalize()))
}

/// Collapse all whitespace runs to single spaces and trim (the S4 needle form).
pub fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The deepest node containing `(start, end)`. For an exact symbol range this is the
/// symbol's own node, never an enclosing node that happens to share the same range (e.g.
/// `source_file` wrapping the file's only item).
fn node_at_range(root: Node, start: usize, end: usize) -> Option<Node> {
    let mut node = root;
    loop {
        let mut cursor = node.walk();
        let mut next = None;
        for child in node.children(&mut cursor) {
            if child.start_byte() <= start && end <= child.end_byte() {
                next = Some(child);
                break;
            }
        }
        match next {
            Some(child) => node = child,
            None => return Some(node),
        }
    }
}

fn serialize(node: Node, source: &str, idents: &mut HashMap<String, usize>, out: &mut String) {
    let kind = node.kind();
    if COMMENT_KINDS.contains(&kind) {
        return;
    }
    if node.child_count() == 0 {
        let text = &source[node.byte_range()];
        if IDENT_KINDS.contains(&kind) {
            let next = idents.len();
            let id = *idents.entry(text.to_string()).or_insert(next);
            out.push_str(&format!("#{id} "));
        } else {
            // keywords, operators, punctuation, primitive_type, and all literals: verbatim
            out.push_str(text);
            out.push(' ');
        }
        return;
    }
    out.push('(');
    out.push_str(kind);
    out.push(' ');
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        serialize(child, source, idents, out);
    }
    out.push(')');
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_across_whitespace_reformat() {
        let a = "fn f(x: i32) -> i32 {\n    let total = x + 1;\n    total\n}";
        let b = "fn f(x: i32)   ->   i32 {\n\n        let total=x+1;\n        total\n}";
        assert_eq!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn stable_across_local_rename() {
        let a = "fn f(x: i32) -> i32 { let total = x + 1; total }";
        let b = "fn f(y: i32) -> i32 { let sum = y + 1; sum }";
        assert_eq!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn stable_across_comment_changes() {
        let a = "fn f() -> i32 { 1 }";
        let b = "/// docs\nfn f() -> i32 {\n    // inner\n    1\n}";
        assert_eq!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn changes_on_literal_change() {
        let a = "fn f(x: i32) -> i32 { x + 1 }";
        let b = "fn f(x: i32) -> i32 { x + 2 }";
        assert_ne!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn changes_on_operator_change() {
        let a = "fn f(x: i32) -> i32 { x + 1 }";
        let b = "fn f(x: i32) -> i32 { x - 1 }";
        assert_ne!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn changes_on_signature_change() {
        let a = "fn f(x: i32) -> i32 { x }";
        let b = "fn f(x: i32, y: i32) -> i32 { x }";
        assert_ne!(fingerprint(a, "f"), fingerprint(b, "f"));
    }

    #[test]
    fn missing_symbol_is_none() {
        assert!(fingerprint("fn f() {}", "nope").is_none());
    }

    #[test]
    fn normalize_ws_collapses() {
        assert_eq!(normalize_ws("fn  f( )\n  {  1 }"), "fn f( ) { 1 }");
    }
}
