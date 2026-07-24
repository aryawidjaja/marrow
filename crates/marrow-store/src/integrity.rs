//! HMAC integrity for memory documents (enterprise tamper-evidence).
//!
//! The signature covers a memory's durable, typed content so any later edit invalidates it.
//! This defends against memory-injection tampering of memories at rest by a party without the key.
//!
//! # Canonicalization
//!
//! The signed bytes are produced by [`canonical_bytes`] from the typed `Memory`, NOT from any
//! serialized document. Encoding is explicit and format-independent: a domain-separated version
//! tag followed by every covered field in a fixed order, each length-prefixed so field boundaries
//! are unambiguous (no canonicalization/extension attack where two distinct memories collide).
//! A memory hydrated from a Postgres row therefore signs identically to the same memory parsed
//! from Markdown — the signature does not depend on YAML field ordering, formatting, or the
//! `serde_yaml` version.
//!
//! ## Covered fields
//!
//! Every durable field of the frontmatter plus the body: `id`, `kind`, `status`, `topic`, `area`,
//! `scope.project_id`, `refs`, `code_anchors`, `confidence`, `decay.expires_at`,
//! `provenance` (`written_by`, `model`, `session_id`, `sources`), `supersedes`, `tags`,
//! `created_at`, `updated_at`, and the (trimmed) `body`. Deliberately EXCLUDED: `hmac` itself
//! (it is the signature). Nothing in `Memory` is a purely-derived index field, so nothing else
//! is excluded. The body is trimmed so persistence normalization (e.g. an added trailing newline
//! on write) does not change the signature across a write/read round-trip.

use hmac::{Hmac, Mac};
use marrow_memdocs::{
    CodeAnchor, Decay, Frontmatter, Memory, MemoryKind, Provenance, Ref, RefKind, Scope, Status,
};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Domain-separation tag; bump the version suffix if the canonical layout ever changes.
const CANON_DOMAIN: &[u8] = b"marrow-memory-canon-v1\0";

/// Accumulates the canonical byte stream with unambiguous, length-prefixed framing.
struct Canonicalizer {
    buf: Vec<u8>,
}

impl Canonicalizer {
    fn new() -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(CANON_DOMAIN);
        Self { buf }
    }

    /// A length-prefixed string field: 8-byte big-endian byte length, then the UTF-8 bytes.
    fn str_field(&mut self, s: &str) -> &mut Self {
        self.buf.extend_from_slice(&(s.len() as u64).to_be_bytes());
        self.buf.extend_from_slice(s.as_bytes());
        self
    }

    /// An optional string: a 1-byte presence tag (0 absent, 1 present), then the string if present.
    fn opt_str_field(&mut self, s: Option<&str>) -> &mut Self {
        match s {
            Some(v) => {
                self.buf.push(1);
                self.str_field(v);
            }
            None => self.buf.push(0),
        }
        self
    }

    /// A count prefix (8-byte big-endian) for a sequence about to be encoded element by element.
    fn count(&mut self, n: usize) -> &mut Self {
        self.buf.extend_from_slice(&(n as u64).to_be_bytes());
        self
    }

    /// A 64-bit float encoded by its raw IEEE-754 big-endian bit pattern, so the same typed value
    /// signs identically regardless of any textual formatting a backend might use.
    fn f64_field(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_bits().to_be_bytes());
        self
    }

    fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Stable, format-independent name for a memory kind (independent of serde's spelling).
fn kind_tag(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Fact => "fact",
        MemoryKind::Decision => "decision",
        MemoryKind::Entity => "entity",
    }
}

/// Stable, format-independent name for a status.
fn status_tag(status: Status) -> &'static str {
    match status {
        Status::Active => "active",
        Status::Superseded => "superseded",
        Status::Deprecated => "deprecated",
    }
}

/// Stable, format-independent name for a reference kind.
fn ref_kind_tag(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Path => "path",
        RefKind::Symbol => "symbol",
        RefKind::Url => "url",
        RefKind::MemoryId => "memory_id",
        RefKind::Commit => "commit",
    }
}

/// Canonical bytes the signature covers. See the module docs for the covered-field list and the
/// framing scheme. This is the single source of truth for what `sign`/`verify` operate over.
fn canonical_bytes(memory: &Memory) -> Vec<u8> {
    // Exhaustive destructuring (no `..`) is intentional: adding a field to any covered struct then
    // becomes a compile error here, forcing a deliberate cover/exclude decision rather than
    // silently leaving the new field unsigned. Do NOT reorder these bindings — the golden test
    // `canonical_form_is_stable` locks the emitted byte order, not the pattern order, but keep them
    // aligned for readability.
    let Frontmatter {
        id,
        kind,
        status,
        topic,
        area,
        scope,
        refs,
        code_anchors,
        confidence,
        decay,
        provenance,
        supersedes,
        tags,
        created_at,
        updated_at,
        hmac: _, // the signature itself, deliberately not signed
    } = &memory.frontmatter;
    let Scope { project_id } = scope;
    let Provenance {
        written_by,
        model,
        session_id,
        sources,
    } = provenance;

    let mut c = Canonicalizer::new();

    c.str_field(id)
        .str_field(kind_tag(*kind))
        .str_field(status_tag(*status))
        .opt_str_field(topic.as_deref())
        .opt_str_field(area.as_deref())
        .str_field(project_id);

    c.count(refs.len());
    for Ref { kind, value } in refs {
        c.str_field(ref_kind_tag(*kind)).str_field(value);
    }

    c.count(code_anchors.len());
    for CodeAnchor {
        file_path,
        symbol,
        snippet,
        fingerprint,
        norm,
    } in code_anchors
    {
        c.str_field(file_path)
            .str_field(symbol)
            .str_field(snippet)
            .str_field(fingerprint)
            .str_field(norm);
    }

    c.f64_field(*confidence);
    let decay_expires = match decay {
        Some(Decay { expires_at }) => expires_at.as_deref(),
        None => None,
    };
    c.opt_str_field(decay_expires);

    c.str_field(written_by)
        .opt_str_field(model.as_deref())
        .opt_str_field(session_id.as_deref());
    c.count(sources.len());
    for s in sources {
        c.str_field(s);
    }

    c.count(supersedes.len());
    for s in supersedes {
        c.str_field(s);
    }

    c.count(tags.len());
    for t in tags {
        c.str_field(t);
    }

    c.str_field(created_at)
        .str_field(updated_at)
        .str_field(memory.body.trim());

    c.finish()
}

/// Legacy canonical form (pre-format-independent): a serde_yaml dump of the frontmatter with
/// `hmac` cleared, plus the trimmed body. Retained so signatures written by older versions still
/// verify. Do NOT use for new signatures.
fn legacy_canonical(memory: &Memory) -> String {
    let mut fm = memory.frontmatter.clone();
    fm.hmac = None;
    let yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    format!("{yaml}\n---\n{}", memory.body.trim())
}

/// HMAC-SHA256 of `bytes` under `key`, hex-encoded. `None` only if HMAC construction rejects the
/// key — which SHA-256 HMAC never does for any key length — so this fails closed instead of panicking.
fn hmac_hex(bytes: &[u8], key: &[u8]) -> Option<String> {
    let mut mac = HmacSha256::new_from_slice(key).ok()?;
    mac.update(bytes);
    Some(hex::encode(mac.finalize().into_bytes()))
}

/// Compute the hex HMAC-SHA256 of a memory under `key`, over the format-independent canonical form.
pub fn sign(memory: &Memory, key: &[u8]) -> String {
    hmac_hex(&canonical_bytes(memory), key).unwrap_or_default()
}

/// True if the HMAC-SHA256 of `bytes` under `key` equals the tag encoded by `stored_hex`.
/// Uses the `hmac` crate's `verify_slice`, whose comparison is constant-time. Fails closed on any
/// error (bad key length — never for HMAC — or non-hex/wrong-length stored tag).
fn hmac_matches(bytes: &[u8], key: &[u8], stored_hex: &str) -> bool {
    let (Ok(mac), Ok(expected)) = (HmacSha256::new_from_slice(key), hex::decode(stored_hex)) else {
        return false;
    };
    mac.chain_update(bytes).verify_slice(&expected).is_ok()
}

/// Verify a memory's stored `hmac` against `key`. False if absent or mismatched.
///
/// Dual-verify for backward compatibility: a stored signature matches if it equals either the
/// current canonical HMAC or the legacy serde_yaml HMAC. Existing stores signed with the old form
/// keep verifying; the store re-signs with the current form on the next write.
pub fn verify(memory: &Memory, key: &[u8]) -> bool {
    let Some(stored) = &memory.frontmatter.hmac else {
        return false;
    };
    hmac_matches(&canonical_bytes(memory), key, stored)
        || hmac_matches(legacy_canonical(memory).as_bytes(), key, stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_memdocs::{
        CodeAnchor, Decay, Frontmatter, MemoryKind, Provenance, Ref, RefKind, Scope, Status,
    };

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

    /// A fully-populated memory exercising every signature-covered field. Used as the golden
    /// fixture so the stability test locks the exact canonical encoding.
    fn fixed_memory() -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: "01FIXED".into(),
                kind: MemoryKind::Decision,
                status: Status::Superseded,
                topic: Some("jwt-expiry".into()),
                area: Some("auth".into()),
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![
                    Ref {
                        kind: RefKind::Url,
                        value: "https://example.com/rfc".into(),
                    },
                    Ref {
                        kind: RefKind::MemoryId,
                        value: "01OTHER".into(),
                    },
                ],
                code_anchors: vec![CodeAnchor {
                    file_path: "src/auth.rs".into(),
                    symbol: "login".into(),
                    snippet: "fn login() {}".into(),
                    fingerprint: "abc123".into(),
                    norm: "fn login".into(),
                }],
                confidence: 0.75,
                decay: Some(Decay {
                    expires_at: Some("2027-01-01T00:00:00Z".into()),
                }),
                provenance: Provenance {
                    written_by: "claude-code".into(),
                    model: Some("claude-opus-4-8".into()),
                    session_id: Some("sess-42".into()),
                    sources: vec!["chat".into(), "rfc".into()],
                },
                supersedes: vec!["01OLD".into()],
                tags: vec!["security".into(), "jwt".into()],
                created_at: "2026-06-06T00:00:00Z".into(),
                updated_at: "2026-06-07T00:00:00Z".into(),
                hmac: Some("ignored-not-covered".into()),
            },
            body: "We use JWT for sessions.\n\nRotated every 24h.\n".into(),
        }
    }

    fn markdown_roundtrip(m: &Memory) -> Memory {
        marrow_memdocs::parse(&marrow_memdocs::to_markdown(m)).expect("markdown round-trip")
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

    #[test]
    fn signature_is_format_independent() {
        let key = b"k";
        let mut m = fixed_memory();
        m.frontmatter.hmac = Some(sign(&m, key));

        // A memory reconstructed by round-tripping through markdown must canonicalize
        // byte-for-byte identically and keep its signature valid.
        let via_markdown = markdown_roundtrip(&m);
        assert_eq!(canonical_bytes(&via_markdown), canonical_bytes(&m));
        assert!(verify(&via_markdown, key));
    }

    #[test]
    fn canonical_form_is_stable() {
        // Golden value: freezes the exact canonical encoding so any accidental future change
        // to the format (field order, separators, enum spelling) fails loudly here.
        let expected = "6d6172726f772d6d656d6f72792d63616e6f6e2d76310000000000000000073031464958454400000000000000086465636973696f6e000000000000000a7375706572736564656401000000000000000a6a77742d65787069727901000000000000000461757468000000000000000464656d6f0000000000000002000000000000000375726c000000000000001768747470733a2f2f6578616d706c652e636f6d2f72666300000000000000096d656d6f72795f6964000000000000000730314f544845520000000000000001000000000000000b7372632f617574682e727300000000000000056c6f67696e000000000000000d666e206c6f67696e2829207b7d00000000000000066162633132330000000000000008666e206c6f67696e3fe8000000000000010000000000000014323032372d30312d30315430303a30303a30305a000000000000000b636c617564652d636f646501000000000000000f636c617564652d6f7075732d342d38010000000000000007736573732d3432000000000000000200000000000000046368617400000000000000037266630000000000000001000000000000000530314f4c4400000000000000020000000000000008736563757269747900000000000000036a77740000000000000014323032362d30362d30365430303a30303a30305a0000000000000014323032362d30362d30375430303a30303a30305a000000000000002c576520757365204a575420666f722073657373696f6e732e0a0a526f7461746564206576657279203234682e";
        assert_eq!(hex::encode(canonical_bytes(&fixed_memory())), expected);
    }

    #[test]
    fn tampering_a_covered_non_body_field_breaks_verification() {
        let key = b"secret";
        let mut m = fixed_memory();
        m.frontmatter.hmac = Some(sign(&m, key));
        // Flip a covered provenance field: the signature must reject it.
        m.frontmatter.confidence = 0.1;
        assert!(!verify(&m, key));
    }

    #[test]
    fn legacy_signed_memory_still_verifies() {
        // Memories signed with the old serde_yaml canonicalization must keep verifying so we
        // never silently invalidate an existing store. `verify` dual-verifies (new, then legacy).
        let key = b"secret";
        let mut m = fixed_memory();
        m.frontmatter.hmac = None;
        let legacy = {
            let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(legacy_canonical(&m).as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };
        m.frontmatter.hmac = Some(legacy);
        assert!(verify(&m, key));
    }
}
