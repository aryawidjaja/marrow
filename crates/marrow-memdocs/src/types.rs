//! Typed frontmatter model for Marrow memory documents.

use serde::{Deserialize, Serialize};

/// The kind of a memory (its base schema).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Fact,
    Decision,
    Entity,
    Session,
    Skill,
}

/// Lifecycle status of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Active,
    Superseded,
    Draft,
    Deprecated,
}

/// Ownership scope. `project_id` is always required; the rest narrow visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_id: Option<String>,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub org_id: Option<String>,
}

/// What kind of thing a reference points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefKind {
    Path,
    Symbol,
    Url,
    MemoryId,
    Commit,
}

/// A reference from a memory to a code location, URL, or another memory.
///
/// When `kind` is `path` or `symbol`, `anchor` may hold a marrow-core fingerprint so the
/// reference can be staleness-checked against the live code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    pub kind: RefKind,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub anchor: Option<String>,
}

/// A code anchor carried by a memory, mirroring a `marrow_core::Anchor` so the store can
/// run the full structural+relocation staleness check against live code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeAnchor {
    pub file_path: String,
    pub symbol: String,
    pub snippet: String,
    pub fingerprint: String,
    pub norm: String,
}

/// Time-based decay configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Decay {
    /// Duration string such as `"30d"` (parsed by the store).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub half_life: Option<String>,
    /// RFC3339 timestamp after which the memory is expired.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires_at: Option<String>,
}

/// Who wrote the memory and from what sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub written_by: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

fn default_confidence() -> f64 {
    1.0
}

/// The YAML frontmatter of a memory document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: MemoryKind,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub topic: Option<String>,
    /// The feature/area this memory belongs to — its one home inside the project (`auth`,
    /// `billing`, `infra`). The agent picks it when writing, reusing the project's existing areas.
    /// Gives the brain a navigable middle layer: project → area → topic → versions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub area: Option<String>,
    pub scope: Scope,
    #[serde(default)]
    pub refs: Vec<Ref>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_anchors: Vec<CodeAnchor>,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub decay: Option<Decay>,
    pub provenance: Provenance,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hmac: Option<String>,
}

/// A full memory document: typed frontmatter plus a markdown body.
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub frontmatter: Frontmatter,
    pub body: String,
}
