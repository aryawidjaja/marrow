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

/// Pull out `file.ext::symbol` (or `file.ext:symbol`) references written in a memory body, so a
/// memory that plainly names the code it is about can be anchored to it automatically. Returns
/// `(file, symbol)` pairs in order of appearance, de-duplicated.
///
/// Deliberately conservative: it requires the JOINED form (path with an extension, then `::`/`:`,
/// then a symbol) — a bare filename or a bare symbol never matches, so only a deliberate "this is
/// about X" reference is picked up. Whether a pair is REAL is settled downstream by [`seed_anchor`],
/// which returns `None` unless the file exists and the symbol resolves; a non-resolving pair is
/// simply dropped, so this never produces a false anchor.
pub fn extract_anchor_refs(body: &str) -> Vec<(String, String)> {
    fn is_path(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
    }
    fn is_sym(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | ':')
    }
    // The last path segment must end in a short alphabetic extension, e.g. `.rs`, `.tsx`.
    fn has_ext(path: &str) -> bool {
        let seg = path.rsplit('/').next().unwrap_or(path);
        match seg.rfind('.') {
            Some(i) => {
                let ext = &seg[i + 1..];
                !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphabetic())
            }
            None => false,
        }
    }

    let chars: Vec<char> = body.chars().collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != ':' {
            i += 1;
            continue;
        }
        // Longest path run ending just before this colon.
        let mut l = i;
        while l > 0 && is_path(chars[l - 1]) {
            l -= 1;
        }
        // Separator is `::` or `:`.
        let sep_len = if i + 1 < chars.len() && chars[i + 1] == ':' {
            2
        } else {
            1
        };
        let sym_start = i + sep_len;
        let mut r = sym_start;
        while r < chars.len() && is_sym(chars[r]) {
            r += 1;
        }
        if l < i && sym_start < r {
            let path: String = chars[l..i].iter().collect();
            let symbol: String = chars[sym_start..r].iter().collect();
            let symbol = symbol.trim_matches(':').to_string();
            if has_ext(&path) && symbol.chars().any(|c| c.is_ascii_alphabetic()) {
                let pair = (path, symbol);
                if seen.insert(pair.clone()) {
                    out.push(pair);
                }
            }
        }
        i = r.max(i + 1);
    }
    out
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

    #[test]
    fn extracts_joined_file_symbol_refs() {
        // `::` and `:` forms, with and without a directory, inside backticks or prose.
        let refs = extract_anchor_refs(
            "The fix is in `crates/marrow-store/src/index.rs::search`; see also config.rs:build.",
        );
        assert!(refs.contains(&(
            "crates/marrow-store/src/index.rs".to_string(),
            "search".to_string()
        )));
        assert!(refs.contains(&("config.rs".to_string(), "build".to_string())));
    }

    #[test]
    fn ignores_bare_names_urls_and_line_numbers() {
        // bare file, bare symbol, a URL, and a file:line — none is a JOINED code ref.
        let refs = extract_anchor_refs(
            "See index.rs and the search function at https://example.com; note.txt:5 line ref.",
        );
        assert!(refs.is_empty(), "should extract nothing, got {refs:?}");
    }

    #[test]
    fn dedupes_repeated_refs() {
        let refs = extract_anchor_refs("a.rs::f and again a.rs::f");
        assert_eq!(refs, vec![("a.rs".to_string(), "f".to_string())]);
    }
}
