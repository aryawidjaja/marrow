//! Cross-file content relocation search (the S4 check).
//!
//! Collapses whitespace in the snippet and in every `.rs` file under the repo root, then
//! tests substring containment. This absorbs reformatting but is intentionally blind to
//! identifier renames (the changed text breaks the match) — the complement of S3.

use std::fs;
use std::path::Path;

use crate::fingerprint::normalize_ws;

/// Search the repo for the normalized snippet. Returns `(relative_path, line)` of the
/// first matching `.rs` file, or `None` if it appears nowhere.
pub fn relocate(repo_root: &Path, norm: &str, snippet: &str) -> Option<(String, usize)> {
    for path in rust_files(repo_root) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if normalize_ws(&text).contains(norm) {
            let rel = path
                .strip_prefix(repo_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            return Some((rel, approx_line(&text, snippet)));
        }
    }
    None
}

/// Best-effort 1-based line of the match: the first line whose trimmed text contains the
/// snippet's first non-empty trimmed line. Falls back to 1.
pub fn approx_line(text: &str, snippet: &str) -> usize {
    let Some(first) = snippet.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return 1;
    };
    text.lines()
        .position(|l| l.trim().contains(first))
        .map(|i| i + 1)
        .unwrap_or(1)
}

/// All `.rs` files under `root`, sorted for deterministic results.
fn rust_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    collect(root, &mut out);
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
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
    fn finds_in_origin_file() {
        let snippet = "fn f(x: i32) -> i32 { x + 1 }";
        let dir = repo(&[("a.rs", "fn f(x: i32) -> i32 { x + 1 }\n")]);
        let hit = relocate(dir.path(), &normalize_ws(snippet), snippet);
        assert_eq!(hit.unwrap().0, "a.rs");
    }

    #[test]
    fn finds_after_cross_file_move() {
        let snippet = "fn f(x: i32) -> i32 { x + 1 }";
        let dir = repo(&[
            ("a.rs", "const Z: u8 = 0;\n"),
            ("b.rs", "struct Other;\nfn f(x: i32) -> i32 { x + 1 }\n"),
        ]);
        let (path, line) = relocate(dir.path(), &normalize_ws(snippet), snippet).unwrap();
        assert_eq!(path, "b.rs");
        assert_eq!(line, 2);
    }

    #[test]
    fn matches_through_whitespace_reformat() {
        let snippet = "fn f(x: i32) -> i32 { x + 1 }";
        let dir = repo(&[("a.rs", "fn f(x: i32)   ->   i32 {\n    x + 1\n}\n")]);
        assert!(relocate(dir.path(), &normalize_ws(snippet), snippet).is_some());
    }

    #[test]
    fn not_found_is_none() {
        let snippet = "fn f(x: i32) -> i32 { x + 1 }";
        let dir = repo(&[("a.rs", "fn other() {}\n")]);
        assert!(relocate(dir.path(), &normalize_ws(snippet), snippet).is_none());
    }
}
