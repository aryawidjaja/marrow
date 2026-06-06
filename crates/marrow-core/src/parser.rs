//! Tree-sitter Rust parser handle.

use tree_sitter::{Language, Parser, Tree};

/// The tree-sitter grammar for Rust.
pub fn rust_language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

/// Parse Rust `source` into a syntax tree, or `None` if the parser can't be configured.
pub fn parse(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&rust_language()).ok()?;
    parser.parse(source, None)
}
