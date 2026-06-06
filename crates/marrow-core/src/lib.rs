//! marrow-core: code-anchored staleness detection.
//!
//! Anchors a memory to a code symbol and decides whether it is still valid after the
//! code changes, by combining a structural AST fingerprint with a cross-file content
//! relocation search and flagging stale only when both agree.

pub mod fingerprint;
pub mod parser;
pub mod symbols;
pub mod types;
