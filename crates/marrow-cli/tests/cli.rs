//! Drive the CLI through parsed args against a temp store and assert on its output.

use clap::Parser;
use marrow_cli::{run, Cli};

/// Run `marrow <args...>` rooted at `root`; return captured stdout (panics on error).
fn ok(root: &std::path::Path, args: &[&str]) -> String {
    let mut full = vec!["marrow", "--root", root.to_str().unwrap()];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    let mut buf = Vec::new();
    run(cli, &mut buf).expect("command succeeds");
    String::from_utf8(buf).unwrap()
}

/// Like `ok`, but expects the command to return an error.
fn err(root: &std::path::Path, args: &[&str]) -> String {
    let mut full = vec!["marrow", "--root", root.to_str().unwrap()];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    let mut buf = Vec::new();
    run(cli, &mut buf).expect_err("command fails")
}

#[test]
fn init_add_read_query_search_flow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let out = ok(root, &["init"]);
    assert!(out.contains("Initialized"));

    let id = ok(
        root,
        &[
            "add",
            "--kind",
            "decision",
            "--topic",
            "auth",
            "We use JWT for sessions.",
        ],
    )
    .trim()
    .to_string();
    assert!(!id.is_empty());

    let read = ok(root, &["read", &id]);
    assert!(read.contains("type: decision"));
    assert!(read.contains("We use JWT for sessions."));

    let q = ok(root, &["query", "--kind", "decision"]);
    assert!(q.contains(&id));
    assert!(q.contains("1 result(s)"));

    let s = ok(root, &["search", "JWT"]);
    assert!(s.contains(&id), "search should find the memory");

    let none = ok(root, &["search", "kubernetes"]);
    assert!(none.contains("0 result(s)"));
}

#[test]
fn status_counts_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(root, &["add", "--kind", "fact", "--topic", "a", "fact one"]);
    ok(root, &["add", "--kind", "fact", "--topic", "b", "fact two"]);
    let status = ok(root, &["status"]);
    assert!(status.contains("total: 2"));
    assert!(status.contains("fact: 2"));
}

#[test]
fn supersede_then_only_new_is_active() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    let old = ok(
        root,
        &["add", "--kind", "decision", "--topic", "auth", "Use JWT."],
    )
    .trim()
    .to_string();
    let new = ok(
        root,
        &[
            "supersede",
            &old,
            "--kind",
            "decision",
            "--topic",
            "auth",
            "Use opaque tokens.",
        ],
    )
    .trim()
    .to_string();
    // Active decision query returns only the new one.
    let q = ok(root, &["query", "--kind", "decision"]);
    assert!(q.contains(&new));
    assert!(q.contains("1 result(s)"), "superseded one is filtered out");
}

#[test]
fn invalid_write_errors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    // A decision with no topic violates the schema.
    let msg = err(
        root,
        &["add", "--kind", "decision", "decision without a topic"],
    );
    assert!(msg.contains("topic"), "got: {msg}");
}

#[test]
fn doctor_reindexes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(root, &["add", "--kind", "fact", "--topic", "x", "a fact"]);
    // Remove the index db, then doctor rebuilds it.
    std::fs::remove_file(root.join(".marrow/.index/sqlite.db")).unwrap();
    let out = ok(root, &["doctor"]);
    assert!(out.contains("Reindexed 1"));
    assert!(ok(root, &["status"]).contains("total: 1"));
}

#[test]
fn list_stale_reports_changed_code() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    // No code anchors yet -> nothing stale.
    let out = ok(root, &["list-stale", "--repo", root.to_str().unwrap()]);
    assert!(out.contains("0 stale anchor(s)"));
}

#[test]
fn anchor_then_staleness_tracks_code_changes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = root.to_str().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/auth.rs"),
        "pub fn issue_token(user: &str) -> String { format!(\"jwt:{user}\") }\n",
    )
    .unwrap();

    ok(root, &["init"]);
    let id = ok(
        root,
        &[
            "anchor",
            "--kind",
            "decision",
            "--topic",
            "auth",
            "--repo",
            repo,
            "--file",
            "src/auth.rs",
            "--symbol",
            "issue_token",
            "Auth issues a signed JWT string.",
        ],
    )
    .trim()
    .to_string();
    assert!(!id.is_empty());

    // Fresh anchor: nothing stale.
    assert!(ok(root, &["list-stale", "--repo", repo]).contains("0 stale anchor(s)"));

    // Change the function's behavior; the anchored memory must be flagged.
    std::fs::write(
        root.join("src/auth.rs"),
        "pub fn issue_token(user: &str) -> String { format!(\"opaque:{user}:v2\") }\n",
    )
    .unwrap();
    let stale = ok(root, &["list-stale", "--repo", repo]);
    assert!(stale.contains("1 stale anchor(s)"), "got: {stale}");
    assert!(stale.contains("issue_token"));
    assert!(stale.contains(&id));
}
