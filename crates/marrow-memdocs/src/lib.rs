//! marrow-memdocs: typed markdown memory documents for Marrow.
//!
//! A memory is a markdown file with YAML frontmatter conforming to one of the base
//! schemas (`fact`, `decision`, `entity`, `session`, `skill`). This crate parses,
//! serializes, and validates those documents.

pub mod document;
pub mod types;
pub mod validate;

pub use document::{parse, to_markdown, ParseError};
pub use types::{
    wiki_refs, CodeAnchor, Decay, Frontmatter, Memory, MemoryKind, Provenance, Ref, RefKind, Scope,
    Status,
};
pub use validate::{validate, Violation};
