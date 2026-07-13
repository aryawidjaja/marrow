//! Conversions between typed enums and their stored string forms.

use marrow_memdocs::{MemoryKind, Status};

/// Stored string for a memory kind.
pub fn kind_str(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Fact => "fact",
        MemoryKind::Decision => "decision",
        MemoryKind::Entity => "entity",
    }
}

/// Parse a stored kind string.
pub fn parse_kind(s: &str) -> Option<MemoryKind> {
    Some(match s {
        "fact" => MemoryKind::Fact,
        "decision" => MemoryKind::Decision,
        "entity" => MemoryKind::Entity,
        // Kinds from older versions carried no rules of their own; read them as plain facts.
        "session" | "skill" => MemoryKind::Fact,
        _ => return None,
    })
}

/// Stored string for a status.
pub fn status_str(status: Status) -> &'static str {
    match status {
        Status::Active => "active",
        Status::Superseded => "superseded",
        Status::Deprecated => "deprecated",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        for k in [MemoryKind::Fact, MemoryKind::Decision, MemoryKind::Entity] {
            assert_eq!(parse_kind(kind_str(k)), Some(k));
        }
        // Kinds written by older versions still load, rather than being dropped on upgrade.
        assert_eq!(parse_kind("session"), Some(MemoryKind::Fact));
        assert_eq!(parse_kind("skill"), Some(MemoryKind::Fact));
    }
}
