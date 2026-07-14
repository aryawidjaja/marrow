//! Typed frontmatter model for Marrow memory documents.

use serde::{Deserialize, Serialize};

/// The kind of a memory (its base schema).
///
/// The aliases keep memories written by older versions readable: their kinds carried no rules of
/// their own, so they load as plain facts rather than being dropped on the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    #[serde(alias = "session", alias = "skill")]
    Fact,
    Decision,
    Entity,
}

/// Lifecycle status of a memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[serde(alias = "draft")]
    Active,
    Superseded,
    Deprecated,
}

/// Which project a memory belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub project_id: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    pub kind: RefKind,
    pub value: String,
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

/// When a memory stops being true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Decay {
    /// RFC3339 timestamp after which the memory is expired.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub expires_at: Option<String>,
}

/// Who wrote the memory and from what sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub written_by: String,
    /// The model that wrote it, e.g. `claude-opus-4-8`. Self-reported: the MCP handshake names the
    /// client, never the model, so only the agent can say which one was thinking.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
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

/// The `[[target]]` links written inside a memory body. Both the MCP and CLI write paths mirror
/// these into `refs`, so the link graph is structured data rather than prose every reader has to
/// re-parse.
pub fn wiki_refs(body: &str) -> Vec<Ref> {
    let mut out: Vec<Ref> = Vec::new();
    for (i, _) in body.match_indices("[[") {
        let rest = &body[i + 2..];
        let Some(j) = rest.find("]]") else { continue };
        let value = rest[..j].trim();
        if value.is_empty() || value.len() > 64 || out.iter().any(|r| r.value == value) {
            continue;
        }
        out.push(Ref {
            kind: RefKind::MemoryId,
            value: value.to_string(),
        });
    }
    out
}
