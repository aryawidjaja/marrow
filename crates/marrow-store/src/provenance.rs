//! Decision provenance: trace a memory back to its origin and forward to how it's been used.
//!
//! For any memory you can see who wrote it and from what sources, what it replaced and what
//! replaced it, and every audit event that touched it — including the retrievals where it
//! informed an answer. Combined with the tamper-evident ledger, this answers the auditor's
//! question: "what knowledge produced this decision, and can I trust the record?"

use marrow_episodic::Event;
use marrow_memdocs::Memory;

use crate::convert::{kind_str, status_str};
use crate::store::{Error, Store};
use crate::Query;

/// A lightweight reference to a related memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRef {
    pub id: String,
    pub kind: String,
    pub topic: Option<String>,
    pub status: String,
}

/// The full provenance of one memory.
#[derive(Debug, Clone)]
pub struct ProvenanceTrail {
    pub id: String,
    pub written_by: String,
    pub sources: Vec<String>,
    /// Memories this one replaced.
    pub supersedes: Vec<MemoryRef>,
    /// Memories that replaced this one.
    pub superseded_by: Vec<MemoryRef>,
    /// Audit events touching this memory, including retrievals that used it.
    pub events: Vec<Event>,
}

impl Store {
    /// Build the provenance trail for a memory, or `None` if it doesn't exist.
    pub fn provenance(&self, id: &str) -> Result<Option<ProvenanceTrail>, Error> {
        let Some(memory) = self.read(id)? else {
            return Ok(None);
        };

        let mut supersedes = Vec::new();
        for sid in &memory.frontmatter.supersedes {
            match self.read(sid)? {
                Some(m) => supersedes.push(memory_ref(&m)),
                None => supersedes.push(missing_ref(sid)),
            }
        }

        let mut superseded_by = Vec::new();
        for row in self.list()? {
            if row.id == id {
                continue;
            }
            if let Some(other) = self.read(&row.id)? {
                if other.frontmatter.supersedes.iter().any(|s| s == id) {
                    superseded_by.push(memory_ref(&other));
                }
            }
        }

        let events = self
            .history()?
            .into_iter()
            .filter(|e| touches(e, id))
            .collect();

        Ok(Some(ProvenanceTrail {
            id: id.to_string(),
            written_by: memory.frontmatter.provenance.written_by.clone(),
            sources: memory.frontmatter.provenance.sources.clone(),
            supersedes,
            superseded_by,
            events,
        }))
    }

    /// Search, and record the retrieval so the answer it informs is traceable. Returns the
    /// recalled memories (same as `search`).
    pub fn recall(&self, text: &str, q: &Query, actor: &str) -> Result<Vec<Memory>, Error> {
        let hits = self.search(text, q)?;
        let ids: Vec<String> = hits.iter().map(|m| m.frontmatter.id.clone()).collect();
        self.log_retrieval(actor, text, &ids)?;
        Ok(hits)
    }
}

/// An event touches a memory if it names it directly, or recalled it in a retrieval.
fn touches(event: &Event, id: &str) -> bool {
    if event.memory_id.as_deref() == Some(id) {
        return true;
    }
    event.kind == "retrieve"
        && event
            .data
            .get("ids")
            .and_then(|v| v.as_array())
            .is_some_and(|ids| ids.iter().any(|x| x.as_str() == Some(id)))
}

fn memory_ref(m: &Memory) -> MemoryRef {
    MemoryRef {
        id: m.frontmatter.id.clone(),
        kind: kind_str(m.frontmatter.kind).to_string(),
        topic: m.frontmatter.topic.clone(),
        status: status_str(m.frontmatter.status).to_string(),
    }
}

fn missing_ref(id: &str) -> MemoryRef {
    MemoryRef {
        id: id.to_string(),
        kind: "unknown".to_string(),
        topic: None,
        status: "missing".to_string(),
    }
}
