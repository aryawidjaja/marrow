//! Discover named symbols in Rust source and locate them by qualified name.

use crate::parser::parse;
use tree_sitter::Node;

/// Item kinds whose `name` field gives a directly-usable symbol name.
const NAMED_ITEMS: &[&str] = &[
    "function_item",
    "struct_item",
    "enum_item",
    "trait_item",
    "union_item",
    "type_item",
    "const_item",
    "static_item",
    "macro_definition",
];

/// Discover symbols as `(qualified_name, (start_byte, end_byte))`, in source order.
///
/// Top-level items use their bare name. `mod m { .. }` nests its contents under `m::`.
/// `impl Type { fn method .. }` emits methods as `Type::method`.
pub fn iter_symbols(source: &str) -> Vec<(String, (usize, usize))> {
    let mut out = Vec::new();
    let Some(tree) = parse(source) else {
        return out;
    };
    walk(tree.root_node(), "", source, &mut out);
    out
}

/// Byte range of the first symbol whose qualified name equals `name`.
pub fn find_symbol(source: &str, name: &str) -> Option<(usize, usize)> {
    iter_symbols(source)
        .into_iter()
        .find(|(qn, _)| qn == name)
        .map(|(_, range)| range)
}

fn name_field(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| source[n.byte_range()].to_string())
}

fn walk(node: Node, prefix: &str, source: &str, out: &mut Vec<(String, (usize, usize))>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if NAMED_ITEMS.contains(&kind) {
            if let Some(name) = name_field(child, source) {
                let qn = join(prefix, &name);
                let r = child.byte_range();
                out.push((qn.clone(), (r.start, r.end)));
                if kind == "macro_definition" {
                    continue;
                }
            }
        } else if kind == "mod_item" {
            if let Some(name) = name_field(child, source) {
                let qn = join(prefix, &name);
                let r = child.byte_range();
                out.push((qn.clone(), (r.start, r.end)));
                if let Some(body) = child.child_by_field_name("body") {
                    walk(body, &qn, source, out);
                }
            }
        } else if kind == "impl_item" {
            if let Some(ty) = child.child_by_field_name("type") {
                let ty_name = source[ty.byte_range()].to_string();
                let impl_prefix = join(prefix, &ty_name);
                if let Some(body) = child.child_by_field_name("body") {
                    walk_impl(body, &impl_prefix, source, out);
                }
            }
        }
    }
}

/// Emit the methods directly inside an `impl` body (functions only).
fn walk_impl(body: Node, prefix: &str, source: &str, out: &mut Vec<(String, (usize, usize))>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            if let Some(name) = name_field(child, source) {
                let r = child.byte_range();
                out.push((join(prefix, &name), (r.start, r.end)));
            }
        }
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_top_level_function_and_struct() {
        let src = "fn alpha() {}\nstruct Beta { x: u8 }\n";
        let names: Vec<_> = iter_symbols(src).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"Beta".to_string()));
    }

    #[test]
    fn qualifies_impl_methods() {
        let src = "struct S;\nimpl S { fn m(&self) {} fn n() {} }\n";
        let names: Vec<_> = iter_symbols(src).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"S::m".to_string()));
        assert!(names.contains(&"S::n".to_string()));
    }

    #[test]
    fn nests_module_contents() {
        let src = "mod m {\n    fn g() {}\n}\n";
        let names: Vec<_> = iter_symbols(src).into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"m".to_string()));
        assert!(names.contains(&"m::g".to_string()));
    }

    #[test]
    fn find_symbol_hit_returns_node_text() {
        let src = "fn alpha() {}\nfn beta() { let _ = 1; }\n";
        let (s, e) = find_symbol(src, "beta").expect("found");
        let text = &src[s..e];
        assert!(text.starts_with("fn beta"));
        assert!(text.contains("let _ = 1"));
    }

    #[test]
    fn find_symbol_miss_is_none() {
        assert!(find_symbol("fn a() {}", "nope").is_none());
    }
}
