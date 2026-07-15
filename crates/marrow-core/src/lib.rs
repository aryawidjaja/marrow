//! marrow-core: code-anchored staleness detection.
//!
//! Anchors a memory to a code symbol and decides whether it is still valid after the
//! code changes, by combining a structural AST fingerprint (S3) with a cross-file content
//! relocation search (S4) and flagging stale only when both agree.
//!
//! ```no_run
//! use std::path::Path;
//! use marrow_core::{seed_anchor, check_anchor};
//!
//! let repo = Path::new(".");
//! if let Some(anchor) = seed_anchor(repo, "src/lib.rs", "seed_anchor") {
//!     let verdict = check_anchor(repo, &anchor);
//!     println!("stale = {}, relocated = {:?}", verdict.stale, verdict.relocated_to);
//! }
//! ```

pub mod anchor;
pub mod fingerprint;
pub mod parser;
pub mod relocation;
pub mod symbols;
pub mod types;

pub use anchor::{check_anchor, extract_anchor_refs, seed_anchor};
pub use fingerprint::fingerprint;
pub use symbols::iter_symbols;
pub use types::{Anchor, Verdict};
