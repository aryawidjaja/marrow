//! marrow-store: the Marrow memory store.
//!
//! Markdown files under `<root>/.marrow/memory` are the source of truth; a SQLite/FTS5
//! index provides structured query and full-text search. Writes are validated against the
//! memdocs schemas, optionally HMAC-signed, and persisted atomically. Code anchors are
//! staleness-checked against live code via marrow-core's S3∧S4 hybrid.

pub mod associative;
pub mod channel;
pub mod config;
pub mod consolidate;
pub mod convert;
pub mod coordinate;
#[cfg(feature = "distill-http")]
pub mod distill_http;
pub mod embed;
#[cfg(feature = "embed-fastembed")]
pub mod embed_fastembed;
#[cfg(feature = "embed-http")]
pub mod embed_http;
pub mod hub;
pub mod index;
pub mod integrity;
pub mod provenance;
pub mod query;
pub mod staleness;
pub mod store;
pub mod util;

pub use associative::{ConnectedRecall, Neighbor};
pub use channel::Message;
pub use config::Config;
pub use consolidate::{
    Cluster, ClusterAction, ConsolidationOutcome, ConsolidationReport, Distiller,
    HeuristicDistiller, Verdict,
};
pub use coordinate::{knowledge_docs, Briefing, Claim, ClaimScope};
pub use hub::{Hub, HubEvent, HubHit, Project};

/// Whether this binary was built with a real semantic-embedding backend (local `fastembed` or
/// `http`). When false, only keyword search is available regardless of config.
pub fn semantic_supported() -> bool {
    cfg!(feature = "embed-fastembed") || cfg!(feature = "embed-http")
}
pub use embed::{EmbedError, Embedder, HashEmbedder};
pub use provenance::{MemoryRef, ProvenanceTrail};
pub use query::Query;
pub use staleness::StaleHit;
pub use store::{Error, Store};
