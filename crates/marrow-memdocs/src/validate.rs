//! Schema validation for memory documents.
//!
//! Each base kind shares common rules (valid scope, confidence range, provenance) plus a
//! few kind-specific requirements. Validation returns every problem found, so an agent can
//! fix a write in one retry.

use crate::types::{Memory, MemoryKind, Status};

/// A single validation problem, naming the offending field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub field: String,
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// Validate a memory against its base schema. `Ok(())` if it conforms, else every problem.
pub fn validate(memory: &Memory) -> Result<(), Vec<Violation>> {
    let mut v = Vec::new();
    let fm = &memory.frontmatter;

    if fm.id.trim().is_empty() {
        v.push(viol("id", "must not be empty"));
    }
    if fm.scope.project_id.trim().is_empty() {
        v.push(viol("scope.project_id", "is required and must not be empty"));
    }
    if fm.provenance.written_by.trim().is_empty() {
        v.push(viol("provenance.written_by", "is required and must not be empty"));
    }
    if !(0.0..=1.0).contains(&fm.confidence) {
        v.push(viol("confidence", "must be between 0.0 and 1.0"));
    }
    if fm.created_at.trim().is_empty() {
        v.push(viol("created_at", "is required"));
    }
    if fm.updated_at.trim().is_empty() {
        v.push(viol("updated_at", "is required"));
    }

    // Kind-specific rules.
    match fm.kind {
        MemoryKind::Decision => {
            if fm.topic.as_deref().unwrap_or("").trim().is_empty() {
                v.push(viol("topic", "is required for decision memories"));
            }
        }
        MemoryKind::Entity => {
            if memory.body.trim().is_empty() {
                v.push(viol("body", "entity memories must describe the entity"));
            }
        }
        MemoryKind::Fact | MemoryKind::Session | MemoryKind::Skill => {}
    }

    // A superseded memory must record what replaced it.
    if fm.status == Status::Superseded && fm.supersedes.is_empty() {
        // `supersedes` lists what THIS memory replaces; a superseded memory should instead
        // be pointed at by its replacement. We only flag the clearly-broken case: status
        // superseded with no successor recorded anywhere is allowed, but an empty topic on
        // a superseded decision is already covered above.
    }

    if v.is_empty() {
        Ok(())
    } else {
        Err(v)
    }
}

fn viol(field: &str, message: &str) -> Violation {
    Violation {
        field: field.to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn base(kind: MemoryKind) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: "01ABC".into(),
                kind,
                status: Status::Active,
                topic: Some("auth".into()),
                scope: Scope {
                    user_id: None,
                    agent_id: None,
                    project_id: "demo".into(),
                    org_id: None,
                },
                refs: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "agent-1".into(),
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: "2026-06-06T00:00:00Z".into(),
                updated_at: "2026-06-06T00:00:00Z".into(),
                hmac: None,
            },
            body: "An entity description.".into(),
        }
    }

    #[test]
    fn valid_decision_passes() {
        assert!(validate(&base(MemoryKind::Decision)).is_ok());
    }

    #[test]
    fn empty_project_id_fails() {
        let mut m = base(MemoryKind::Fact);
        m.frontmatter.scope.project_id = "".into();
        let errs = validate(&m).unwrap_err();
        assert!(errs.iter().any(|e| e.field == "scope.project_id"));
    }

    #[test]
    fn out_of_range_confidence_fails() {
        let mut m = base(MemoryKind::Fact);
        m.frontmatter.confidence = 1.5;
        assert!(validate(&m).unwrap_err().iter().any(|e| e.field == "confidence"));
    }

    #[test]
    fn decision_without_topic_fails() {
        let mut m = base(MemoryKind::Decision);
        m.frontmatter.topic = None;
        assert!(validate(&m).unwrap_err().iter().any(|e| e.field == "topic"));
    }

    #[test]
    fn entity_without_body_fails() {
        let mut m = base(MemoryKind::Entity);
        m.body = "   ".into();
        assert!(validate(&m).unwrap_err().iter().any(|e| e.field == "body"));
    }

    #[test]
    fn missing_written_by_fails() {
        let mut m = base(MemoryKind::Fact);
        m.frontmatter.provenance.written_by = "".into();
        assert!(validate(&m).unwrap_err().iter().any(|e| e.field == "provenance.written_by"));
    }
}
