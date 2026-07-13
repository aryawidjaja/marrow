//! Staleness checks for a memory's code anchors, via the marrow-core S3∧S4 hybrid.

use std::path::Path;

use marrow_core::{check_anchor, Anchor};
use marrow_memdocs::{CodeAnchor, Memory};

/// A code anchor that no longer matches the live code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleHit {
    pub memory_id: String,
    pub symbol: String,
    pub file_path: String,
    /// Where the symbol moved to, if it was relocated rather than truly stale.
    pub relocated_to: Option<String>,
}

fn to_core(anchor: &CodeAnchor) -> Anchor {
    Anchor {
        file_path: anchor.file_path.clone(),
        symbol: anchor.symbol.clone(),
        snippet: anchor.snippet.clone(),
        fingerprint: anchor.fingerprint.clone(),
        norm: anchor.norm.clone(),
    }
}

/// Return one [`StaleHit`] for each of the memory's code anchors that is now stale.
pub fn check_memory(repo_root: &Path, memory: &Memory) -> Vec<StaleHit> {
    let id = &memory.frontmatter.id;
    memory
        .frontmatter
        .code_anchors
        .iter()
        .filter_map(|ca| {
            let verdict = check_anchor(repo_root, &to_core(ca));
            verdict.stale.then(|| StaleHit {
                memory_id: id.clone(),
                symbol: ca.symbol.clone(),
                file_path: ca.file_path.clone(),
                relocated_to: verdict.relocated_to,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_core::seed_anchor;
    use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope, Status};
    use std::fs;

    fn memory_with_anchor(repo: &Path) -> Memory {
        // Seed a real anchor from on-disk code so the fields are consistent.
        let core = seed_anchor(repo, "src/lib.rs", "Calc::add").expect("seed");
        let ca = CodeAnchor {
            file_path: core.file_path,
            symbol: core.symbol,
            snippet: core.snippet,
            fingerprint: core.fingerprint,
            norm: core.norm,
        };
        Memory {
            frontmatter: Frontmatter {
                id: "01".into(),
                kind: MemoryKind::Decision,
                status: Status::Active,
                topic: Some("calc".into()),
                area: None,
                scope: Scope {
                    user_id: None,
                    agent_id: None,
                    project_id: "demo".into(),
                    org_id: None,
                },
                refs: vec![],
                code_anchors: vec![ca],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "a".into(),
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: "2026-06-06T00:00:00Z".into(),
                updated_at: "2026-06-06T00:00:00Z".into(),
                hmac: None,
            },
            body: "Calc::add sums two ints".into(),
        }
    }

    const SRC: &str =
        "pub struct Calc;\nimpl Calc {\n    pub fn add(&self, x: i32, y: i32) -> i32 { x + y }\n}\n";

    #[test]
    fn unchanged_code_has_no_stale_hits() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), SRC).unwrap();
        let m = memory_with_anchor(dir.path());
        assert!(check_memory(dir.path(), &m).is_empty());
    }

    #[test]
    fn changed_logic_produces_stale_hit() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), SRC).unwrap();
        let m = memory_with_anchor(dir.path());
        // Change the body so both S3 and S4 see it as gone.
        fs::write(
            dir.path().join("src/lib.rs"),
            "pub struct Calc;\nimpl Calc {\n    pub fn add(&self, x: i32, y: i32) -> i32 { x * y - 99 }\n}\n",
        )
        .unwrap();
        let hits = check_memory(dir.path(), &m);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].symbol, "Calc::add");
    }
}
