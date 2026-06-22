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

#[test]
fn search_accepts_weight_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(
        root,
        &["add", "--kind", "fact", "--topic", "x", "alpha beta"],
    );
    let hits = ok(root, &["search", "alpha", "--weight", "0"]);
    assert!(hits.contains("1 result(s)"));
}

#[test]
fn history_and_audit_track_writes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(root, &["add", "--kind", "fact", "--topic", "x", "a fact"]);
    ok(root, &["log", "--kind", "observe", "saw something"]);

    let hist = ok(root, &["history"]);
    assert!(hist.contains("write"));
    assert!(hist.contains("observe"));
    assert!(hist.contains("2 event(s)"));

    let audit = ok(root, &["audit"]);
    assert!(audit.contains("audit ok"));
    assert!(audit.contains("chain intact"));
}

#[test]
fn consolidate_reports_and_applies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let repo = root.to_str().unwrap();
    ok(root, &["init"]);
    ok(
        root,
        &["add", "--kind", "fact", "--topic", "a", "the disk is full"],
    );
    ok(
        root,
        &["add", "--kind", "fact", "--topic", "b", "the disk is full"],
    );

    let report = ok(root, &["consolidate", "--repo", repo]);
    assert!(report.contains("related memories: 1"));

    let applied = ok(root, &["consolidate", "--repo", repo, "--apply"]);
    assert!(applied.contains("1 merged"));
    // After applying, no duplicates remain.
    assert!(ok(root, &["consolidate", "--repo", repo]).contains("related memories: 0"));
}

#[test]
fn recall_and_provenance_trace_usage() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    let id = ok(
        root,
        &[
            "add",
            "--kind",
            "fact",
            "--topic",
            "limits",
            "rate limit is 100 per minute",
        ],
    )
    .trim()
    .to_string();
    // recall records a retrieval
    let hits = ok(root, &["recall", "rate limit", "--by", "agent"]);
    assert!(hits.contains(&id));
    // provenance shows the write and the retrieval
    let prov = ok(root, &["provenance", &id]);
    assert!(prov.contains("written by"));
    assert!(prov.contains("retrieve"));
}

#[test]
fn claim_release_and_bootstrap_flow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);

    // Agent A claims the auth refactor on a file.
    let claim_id = ok(
        root,
        &[
            "claim",
            "refactor auth",
            "--session",
            "a",
            "--file",
            "src/auth.rs",
            "--project",
            "demo",
        ],
    )
    .trim()
    .to_string();
    assert!(!claim_id.is_empty());

    // Agent B checks the same file and sees the collision.
    let overlap = ok(
        root,
        &["claims", "--file", "src/auth.rs", "--project", "demo"],
    );
    assert!(overlap.contains("refactor auth"));
    assert!(overlap.contains("1 active claim(s)"));

    // Activity shows the claim event.
    let act = ok(root, &["activity"]);
    assert!(act.contains("claim"));

    // Release it; no claims remain.
    ok(root, &["release", &claim_id]);
    let after = ok(root, &["claims"]);
    assert!(after.contains("0 active claim(s)"));

    // Bootstrap announces a session and prints the (now empty) claim list.
    let brief = ok(root, &["bootstrap", "set up auth", "--project", "demo"]);
    assert!(brief.contains("goal: set up auth"));
    assert!(brief.contains("active claims (0)"));
}

#[test]
fn setup_scaffolds_hooks_settings_and_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["setup"]); // claude CLI may be absent here; setup still scaffolds files

    assert!(root.join(".claude/hooks/marrow-bootstrap.sh").exists());
    assert!(root.join(".claude/hooks/marrow-guard.sh").exists());
    assert!(root.join(".claude/hooks/marrow-progress.sh").exists());
    assert!(root.join(".claude/settings.json").exists());
    assert!(root.join(".claude/commands/marrow-save.md").exists());
    let claude_md = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
    assert!(
        claude_md.contains("marrow:begin"),
        "guidance block added to CLAUDE.md"
    );
}

#[test]
fn embed_sets_backend_and_status_reflects_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);

    // Default build has no semantic support, so status reports keyword search.
    assert!(ok(root, &["status"]).contains("search: keyword"));

    let out = ok(root, &["embed", "fastembed"]);
    assert!(out.contains("set to 'fastembed'"));
    // Config persisted.
    let cfg = std::fs::read_to_string(root.join(".marrow/.marrow.toml")).unwrap();
    assert!(cfg.contains("provider = \"fastembed\""));
    // Without the feature compiled in, status still shows semantic provider configured.
    assert!(ok(root, &["status"]).contains("search: semantic"));
}

#[test]
fn bare_invocation_shows_getting_started() {
    let dir = tempfile::tempdir().unwrap();
    let out = ok(dir.path(), &[]);
    assert!(out.contains("shared memory for AI agents"));
    assert!(out.contains("marrow setup"));
}

#[test]
fn ingest_lists_docs_with_distill_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    std::fs::write(root.join("README.md"), "# readme").unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/architecture.md"), "arch").unwrap();

    let out = ok(root, &["ingest"]);
    assert!(out.contains("README.md"));
    assert!(out.contains("docs/architecture.md"));
    assert!(out.contains("mem_write"));
    assert!(out.contains("distill"));

    // A project with no docs says so plainly.
    let empty = tempfile::tempdir().unwrap();
    ok(empty.path(), &["init"]);
    assert!(ok(empty.path(), &["ingest"]).contains("No knowledge docs"));
}

#[test]
fn consolidate_if_due_runs_only_past_threshold_and_bootstrap_nudges() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);

    // A few writes: not due, no nudge.
    for i in 0..3 {
        ok(
            root,
            &[
                "add",
                "--kind",
                "fact",
                "--topic",
                &format!("t{i}"),
                &format!("body {i}"),
            ],
        );
    }
    assert!(ok(root, &["consolidate", "--if-due"]).contains("not due"));
    assert!(!ok(root, &["bootstrap", "resume", "--project", "demo"]).contains("maintenance:"));

    // Cross the threshold (20 writes since last consolidation).
    for i in 3..20 {
        ok(
            root,
            &[
                "add",
                "--kind",
                "fact",
                "--topic",
                &format!("t{i}"),
                &format!("body {i}"),
            ],
        );
    }
    let brief = ok(root, &["bootstrap", "resume", "--project", "demo"]);
    assert!(brief.contains("maintenance:"));
    assert!(brief.contains("marrow consolidate"));

    // Now --if-due applies, and a second call is a no-op again (counter reset).
    assert!(ok(root, &["consolidate", "--if-due"]).contains("applied:"));
    assert!(ok(root, &["consolidate", "--if-due"]).contains("not due"));
}

#[test]
fn watch_shows_other_sessions_then_advances_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    ok(
        root,
        &[
            "claim",
            "refactor auth",
            "--session",
            "other",
            "--file",
            "src/auth.rs",
            "--project",
            "demo",
        ],
    );
    ok(
        root,
        &[
            "progress",
            "edited the parser",
            "--session",
            "other",
            "--file",
            "src/parser.rs",
        ],
    );

    let first = ok(root, &["watch", "--session", "me"]);
    assert!(first.contains("refactor auth"));
    assert!(first.contains("edited the parser"));

    // Watermark advanced: the deltas are gone, but the still-active claim is noted.
    let second = ok(root, &["watch", "--session", "me"]);
    assert!(!second.contains("edited the parser"));
    assert!(second.contains("other session(s) hold active claims"));

    // My own activity never shows up as someone else's.
    ok(
        root,
        &[
            "progress",
            "my own edit",
            "--session",
            "me",
            "--file",
            "src/me.rs",
        ],
    );
    assert!(!ok(root, &["watch", "--session", "me"]).contains("my own edit"));
}

#[test]
fn bootstrap_nudges_ingest_when_empty_with_docs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init"]);
    std::fs::write(root.join("README.md"), "# readme").unwrap();

    let brief = ok(root, &["bootstrap", "resume", "--project", "demo"]);
    assert!(brief.contains("onboarding"));
    assert!(brief.contains("marrow ingest"));
}
