//! HMAC integrity for memory documents (enterprise tamper-evidence).
//!
//! The signature covers the canonical content of a memory — its frontmatter (with the
//! `hmac` field cleared) plus its body — so any later edit invalidates it. This defends
//! against memory-injection tampering of files at rest by a party without the key.

use hmac::{Hmac, Mac};
use marrow_memdocs::Memory;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Canonical bytes that the signature covers.
///
/// The body is trimmed so that persistence normalization (e.g. an added trailing newline
/// on write) does not change the signature across a write/read round-trip.
fn canonical(memory: &Memory) -> String {
    let mut fm = memory.frontmatter.clone();
    fm.hmac = None;
    let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    format!("{yaml}\n---\n{}", memory.body.trim())
}

/// Compute the hex HMAC-SHA256 of a memory under `key`.
pub fn sign(memory: &Memory, key: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(canonical(memory).as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Verify a memory's stored `hmac` against `key`. False if absent or mismatched.
pub fn verify(memory: &Memory, key: &[u8]) -> bool {
    match &memory.frontmatter.hmac {
        Some(stored) => sign(memory, key) == *stored,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope, Status};

    fn mem() -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: "01".into(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: None,
                area: None,
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "a".into(),
                    model: None,
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: "2026-06-06T00:00:00Z".into(),
                updated_at: "2026-06-06T00:00:00Z".into(),
                hmac: None,
            },
            body: "hello".into(),
        }
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let key = b"secret";
        let mut m = mem();
        m.frontmatter.hmac = Some(sign(&m, key));
        assert!(verify(&m, key));
    }

    #[test]
    fn tamper_breaks_verification() {
        let key = b"secret";
        let mut m = mem();
        m.frontmatter.hmac = Some(sign(&m, key));
        m.body = "tampered".into();
        assert!(!verify(&m, key));
    }

    #[test]
    fn wrong_key_fails() {
        let mut m = mem();
        m.frontmatter.hmac = Some(sign(&m, b"secret"));
        assert!(!verify(&m, b"other"));
    }
}
