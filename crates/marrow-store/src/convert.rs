//! Conversions between typed enums and their stored string forms.

use marrow_memdocs::{MemoryKind, Status};

/// Stored string for a memory kind.
pub fn kind_str(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Fact => "fact",
        MemoryKind::Decision => "decision",
        MemoryKind::Entity => "entity",
        MemoryKind::Session => "session",
        MemoryKind::Skill => "skill",
    }
}

/// Parse a stored kind string.
pub fn parse_kind(s: &str) -> Option<MemoryKind> {
    Some(match s {
        "fact" => MemoryKind::Fact,
        "decision" => MemoryKind::Decision,
        "entity" => MemoryKind::Entity,
        "session" => MemoryKind::Session,
        "skill" => MemoryKind::Skill,
        _ => return None,
    })
}

/// Stored string for a status.
pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Active => "active",
        Status::Superseded => "superseded",
        Status::Draft => "draft",
        Status::Deprecated => "deprecated",
    }
}

/// Parse a stored status string.
pub fn parse_status(s: &str) -> Option<Status> {
    Some(match s {
        "active" => Status::Active,
        "superseded" => Status::Superseded,
        "draft" => Status::Draft,
        "deprecated" => Status::Deprecated,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        for k in [
            MemoryKind::Fact,
            MemoryKind::Decision,
            MemoryKind::Entity,
            MemoryKind::Session,
            MemoryKind::Skill,
        ] {
            assert_eq!(parse_kind(kind_str(k)), Some(k));
        }
    }

    #[test]
    fn status_round_trips() {
        for s in [
            Status::Active,
            Status::Superseded,
            Status::Draft,
            Status::Deprecated,
        ] {
            assert_eq!(parse_status(status_str(s)), Some(s));
        }
    }
}
