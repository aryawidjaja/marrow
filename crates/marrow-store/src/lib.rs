//! marrow-store: the Marrow memory store.
//!
//! Markdown files under `<root>/.marrow/memory` are the source of truth; a SQLite/FTS5
//! index provides structured query and full-text search. Writes are validated against the
//! memdocs schemas, optionally HMAC-signed, and persisted atomically. Code anchors are
//! staleness-checked against live code via marrow-core's S3∧S4 hybrid.

pub mod config;
pub mod convert;
pub mod embed;
#[cfg(feature = "embed-fastembed")]
pub mod embed_fastembed;
#[cfg(feature = "embed-http")]
pub mod embed_http;
pub mod index;
pub mod integrity;
pub mod query;
pub mod staleness;
pub mod store;
pub mod util;

pub use config::Config;
pub use embed::{EmbedError, Embedder, HashEmbedder};
pub use query::Query;
pub use staleness::StaleHit;
pub use store::{Error, Store};
