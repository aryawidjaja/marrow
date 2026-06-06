//! Seed anchors and check them against the current repo (the S3 ∧ S4 hybrid).

use std::fs;
use std::path::Path;

use crate::fingerprint::{fingerprint, normalize_ws};
use crate::relocation::relocate;
use crate::symbols::find_symbol;
use crate::types::{Anchor, Verdict};

/// Build an anchor for `symbol` in `repo_root/file_path` from the current source.
///
/// Returns `None` if the file can't be read or parsed, or the symbol isn't found.
pub fn seed_anchor(repo_root: &Path, file_path: &str, symbol: &str) -> Option<Anchor> {
    let source = fs::read_to_string(repo_root.join(file_path)).ok()?;
    let (start, end) = find_symbol(&source, symbol)?;
    let snippet = source[start..end].to_string();
    let fp = fingerprint(&source, symbol)?;
    let norm = normalize_ws(&snippet);
    Some(Anchor {
        file_path: file_path.to_string(),
        symbol: symbol.to_string(),
        snippet,
        fingerprint: fp,
        norm,
    })
}

/// Decide whether `anchor` has gone stale, combining the structural (S3) and relocation
/// (S4) checks: stale only when *both* agree. A cross-file move is reported via
/// `relocated_to` and is not stale.
pub fn check_anchor(repo_root: &Path, anchor: &Anchor) -> Verdict {
    let s3_stale = !structurally_present(repo_root, anchor);

    let relocation = relocate(repo_root, &anchor.norm, &anchor.snippet);
    let s4_stale = relocation.is_none();

    let relocated_to = relocation.and_then(|(path, line)| {
        if path == anchor.file_path {
            None
        } else {
            Some(format!("{path}:{line}"))
        }
    });

    Verdict {
        stale: s3_stale && s4_stale,
        relocated_to,
    }
}

/// True if the symbol still exists at its recorded path with an unchanged fingerprint.
fn structurally_present(repo_root: &Path, anchor: &Anchor) -> bool {
    let Ok(source) = fs::read_to_string(repo_root.join(&anchor.file_path)) else {
        return false;
    };
    fingerprint(&source, &anchor.symbol).is_some_and(|fp| fp == anchor.fingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, src) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, src).unwrap();
        }
        dir
    }

    #[test]
    fn seed_then_unchanged_is_not_stale() {
        let dir = repo(&[("a.rs", "fn f(x: i32) -> i32 { x + 1 }\n")]);
        let anchor = seed_anchor(dir.path(), "a.rs", "f").expect("seed");
        let v = check_anchor(dir.path(), &anchor);
        assert!(!v.stale);
        assert!(v.relocated_to.is_none());
    }

    #[test]
    fn seed_missing_symbol_is_none() {
        let dir = repo(&[("a.rs", "fn f() {}\n")]);
        assert!(seed_anchor(dir.path(), "a.rs", "nope").is_none());
    }

    #[test]
    fn seed_captures_snippet_and_fingerprint() {
        let dir = repo(&[("a.rs", "fn f(x: i32) -> i32 { x + 1 }\n")]);
        let anchor = seed_anchor(dir.path(), "a.rs", "f").unwrap();
        assert!(anchor.snippet.starts_with("fn f"));
        assert!(!anchor.fingerprint.is_empty());
        assert_eq!(anchor.norm, "fn f(x: i32) -> i32 { x + 1 }");
    }
}
