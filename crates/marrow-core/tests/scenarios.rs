//! End-to-end staleness scenarios: seed an anchor, edit the code, check the verdict.
//!
//! These exercise the S3 ∧ S4 hybrid through the public API across every mutation
//! category. `stale` is the positive class.

use std::fs;
use std::path::Path;

use marrow_core::{check_anchor, seed_anchor};

/// Write `files` into a fresh temp repo and return it.
fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_files(dir.path(), files);
    dir
}

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (rel, src) in files {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, src).unwrap();
    }
}

const ORIGINAL: &str = "\
pub struct Calc;

impl Calc {
    pub fn add(&self, x: i32, y: i32) -> i32 {
        let total = x + y;
        total
    }
}
";

/// Seed an anchor on `Calc::add`, then overwrite the file with `mutated`.
fn seed_then_mutate(mutated: &str) -> (tempfile::TempDir, marrow_core::Anchor) {
    let dir = repo(&[("src/lib.rs", ORIGINAL)]);
    let anchor = seed_anchor(dir.path(), "src/lib.rs", "Calc::add").expect("seed");
    fs::write(dir.path().join("src/lib.rs"), mutated).unwrap();
    (dir, anchor)
}

#[test]
fn control_no_change_is_not_stale() {
    let (dir, anchor) = seed_then_mutate(ORIGINAL);
    assert!(!check_anchor(dir.path(), &anchor).stale);
}

#[test]
fn reformat_is_not_stale() {
    let mutated = "\
pub struct Calc;

impl Calc {
    // recalculation helper
    pub fn add(&self,   x: i32,   y: i32)   ->   i32 {

        let total =
            x + y;

        total
    }
}
";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        !check_anchor(dir.path(), &anchor).stale,
        "reformat should be valid"
    );
}

#[test]
fn rename_local_is_not_stale() {
    let mutated = "\
pub struct Calc;

impl Calc {
    pub fn add(&self, a: i32, b: i32) -> i32 {
        let sum = a + b;
        sum
    }
}
";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        !check_anchor(dir.path(), &anchor).stale,
        "renaming locals/params should be valid (S3 holds)"
    );
}

#[test]
fn add_adjacent_item_is_not_stale() {
    let mutated = "\
pub struct Calc;

impl Calc {
    pub fn helper(&self) -> i32 { 0 }

    pub fn add(&self, x: i32, y: i32) -> i32 {
        let total = x + y;
        total
    }
}
";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        !check_anchor(dir.path(), &anchor).stale,
        "unrelated neighbor should be valid"
    );
}

#[test]
fn move_to_another_file_is_not_stale_and_relocates() {
    // Remove from the origin file, add an identical copy in another file.
    let dir = repo(&[("src/lib.rs", ORIGINAL)]);
    let anchor = seed_anchor(dir.path(), "src/lib.rs", "Calc::add").expect("seed");
    fs::write(dir.path().join("src/lib.rs"), "pub struct Calc;\n").unwrap();
    fs::write(
        dir.path().join("src/moved.rs"),
        format!("pub struct Calc;\n{}", &anchor.snippet),
    )
    .unwrap();

    let verdict = check_anchor(dir.path(), &anchor);
    assert!(!verdict.stale, "a cross-file move should not be stale");
    let relocated = verdict.relocated_to.expect("relocation reported");
    assert!(relocated.starts_with("src/moved.rs:"), "got {relocated}");
}

#[test]
fn change_logic_is_stale() {
    let mutated = "\
pub struct Calc;

impl Calc {
    pub fn add(&self, x: i32, y: i32) -> i32 {
        let total = x - y;
        total
    }
}
";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        check_anchor(dir.path(), &anchor).stale,
        "operator change should be stale"
    );
}

#[test]
fn change_signature_is_stale() {
    let mutated = "\
pub struct Calc;

impl Calc {
    pub fn add(&self, x: i32, y: i32, z: i32) -> i32 {
        let total = x + y;
        total
    }
}
";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        check_anchor(dir.path(), &anchor).stale,
        "added parameter should be stale"
    );
}

#[test]
fn delete_symbol_is_stale() {
    let mutated = "pub struct Calc;\n";
    let (dir, anchor) = seed_then_mutate(mutated);
    assert!(
        check_anchor(dir.path(), &anchor).stale,
        "deletion should be stale"
    );
}
