//! End-to-end store behavior: write, read, query, search, supersede, validation,
//! uniqueness, decay/expiry, token budget, HMAC, staleness, and reindex.

use std::fs;
use std::path::Path;

use marrow_core::seed_anchor;
use marrow_memdocs::{
    CodeAnchor, Decay, Frontmatter, Memory, MemoryKind, Provenance, Ref, RefKind, Scope, Status,
};
use marrow_store::{Error, Query, Store};

fn mem(kind: MemoryKind, topic: &str, body: &str) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(), // store assigns a ULID
            kind,
            status: Status::Active,
            topic: Some(topic.into()),
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: "demo".into(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: "agent-1".into(),
                session_id: None,
                sources: vec![],
            },
            supersedes: vec![],
            tags: vec![],
            created_at: String::new(),
            updated_at: String::new(),
            hmac: None,
        },
        body: body.into(),
    }
}

#[test]
fn write_read_round_trips_and_assigns_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();

    let mut m = mem(
        MemoryKind::Fact,
        "auth",
        "We rotate API keys every 90 days.",
    );
    let id = store.write(&mut m).unwrap();
    assert!(!id.is_empty(), "an id should be assigned");

    let got = store.read(&id).unwrap().expect("memory present");
    assert_eq!(got.body.trim(), "We rotate API keys every 90 days.");
    assert_eq!(got.frontmatter.scope.project_id, "demo");
    // The file exists under the kind directory.
    assert!(dir
        .path()
        .join(format!(".marrow/memory/fact/{id}.md"))
        .exists());
}

#[test]
fn query_and_search_find_memories() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(
        MemoryKind::Decision,
        "rate-limits",
        "EHR endpoints use a token bucket.",
    );
    let id = store.write(&mut m).unwrap();

    let q = Query {
        kind: Some(MemoryKind::Decision),
        status: Some(Status::Active),
        project_id: Some("demo".into()),
        ..Query::default()
    };
    let hits = store.query(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].frontmatter.id, id);

    let found = store
        .search("token bucket", &Query::for_project("demo"))
        .unwrap();
    assert_eq!(found.len(), 1, "FTS should match the body text");

    let none = store
        .search("nonexistent phrase", &Query::for_project("demo"))
        .unwrap();
    assert!(none.is_empty());
}

#[test]
fn unique_active_decision_per_topic_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Decision, "auth", "Use JWT.");
    store.write(&mut a).unwrap();

    let mut b = mem(MemoryKind::Decision, "auth", "Use sessions.");
    match store.write(&mut b) {
        Err(Error::Conflict(_)) => {}
        other => panic!("expected conflict, got {other:?}"),
    }
}

#[test]
fn supersede_marks_old_and_links_new() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut old = mem(MemoryKind::Decision, "auth", "Use JWT.");
    let old_id = store.write(&mut old).unwrap();

    let mut new = mem(MemoryKind::Decision, "auth", "Use opaque tokens.");
    let new_id = store.supersede(&old_id, &mut new).unwrap();

    let old_now = store.read(&old_id).unwrap().unwrap();
    assert_eq!(old_now.frontmatter.status, Status::Superseded);
    let new_now = store.read(&new_id).unwrap().unwrap();
    assert_eq!(new_now.frontmatter.status, Status::Active);
    assert!(new_now.frontmatter.supersedes.contains(&old_id));
}

#[test]
fn invalid_memory_is_rejected_with_violations() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut bad = mem(MemoryKind::Fact, "x", "body");
    bad.frontmatter.provenance.written_by = String::new();
    match store.write(&mut bad) {
        Err(Error::Invalid(v)) => assert!(v.iter().any(|e| e.field == "provenance.written_by")),
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[test]
fn expired_memories_are_excluded_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "temp", "ephemeral note");
    m.frontmatter.decay = Some(Decay {
        half_life: None,
        expires_at: Some("2000-01-01T00:00:00Z".into()),
    });
    store.write(&mut m).unwrap();

    assert!(store.query(&Query::for_project("demo")).unwrap().is_empty());

    let include = Query {
        project_id: Some("demo".into()),
        exclude_expired: false,
        ..Query::default()
    };
    assert_eq!(store.query(&include).unwrap().len(), 1);
}

#[test]
fn token_budget_limits_results() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    for i in 0..3 {
        let mut m = mem(MemoryKind::Fact, &format!("t{i}"), &"word ".repeat(40));
        store.write(&mut m).unwrap();
    }
    let q = Query {
        project_id: Some("demo".into()),
        max_tokens: Some(60), // ~one body of 40 words (~50 tokens)
        ..Query::default()
    };
    let hits = store.query(&q).unwrap();
    assert!(
        hits.len() < 3,
        "budget should cut results, got {}",
        hits.len()
    );
    assert!(!hits.is_empty(), "at least one result is always returned");
}

#[test]
fn hmac_signing_round_trips_and_detects_tamper() {
    let dir = tempfile::tempdir().unwrap();
    // Enable signing in config before opening.
    Store::init(dir.path()).unwrap();
    fs::write(
        dir.path().join(".marrow/.marrow.toml"),
        "project_id = \"demo\"\nsign = true\n",
    )
    .unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    store.set_key(b"super-secret".to_vec());

    let mut m = mem(MemoryKind::Fact, "auth", "signed fact");
    let id = store.write(&mut m).unwrap();

    let stored = store.read(&id).unwrap().unwrap();
    assert!(stored.frontmatter.hmac.is_some());
    assert_eq!(store.verify(&stored), Some(true));

    let mut tampered = stored.clone();
    tampered.body = "tampered".into();
    assert_eq!(store.verify(&tampered), Some(false));
}

const CODE: &str =
    "pub struct Calc;\nimpl Calc {\n    pub fn add(&self, x: i32, y: i32) -> i32 { x + y }\n}\n";

fn write_code(repo: &Path) {
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/lib.rs"), CODE).unwrap();
}

#[test]
fn list_stale_flags_changed_code_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    // The "repo" being referenced lives alongside the store.
    write_code(dir.path());
    let core = seed_anchor(dir.path(), "src/lib.rs", "Calc::add").unwrap();

    let mut m = mem(MemoryKind::Decision, "calc", "Calc::add sums two integers.");
    m.frontmatter.refs.push(Ref {
        kind: RefKind::Symbol,
        value: "src/lib.rs::Calc::add".into(),
        anchor: Some(core.fingerprint.clone()),
    });
    m.frontmatter.code_anchors.push(CodeAnchor {
        file_path: core.file_path,
        symbol: core.symbol,
        snippet: core.snippet,
        fingerprint: core.fingerprint,
        norm: core.norm,
    });
    store.write(&mut m).unwrap();

    assert!(
        store.list_stale(dir.path()).unwrap().is_empty(),
        "fresh anchor is not stale"
    );

    // Materially change the function so both checks see it gone.
    fs::write(
        dir.path().join("src/lib.rs"),
        "pub struct Calc;\nimpl Calc {\n    pub fn add(&self, x: i32, y: i32) -> i32 { x * y - 7 }\n}\n",
    )
    .unwrap();
    let stale = store.list_stale(dir.path()).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].symbol, "Calc::add");
}

#[test]
fn reindex_rebuilds_from_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "auth", "rebuildable index entry");
    let id = store.write(&mut m).unwrap();

    // Wipe the index database, reopen, and rebuild from the markdown files.
    drop(store);
    fs::remove_file(dir.path().join(".marrow/.index/sqlite.db")).unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(
        store.read(&id).unwrap().is_none(),
        "index is empty before reindex"
    );

    let count = store.reindex().unwrap();
    assert_eq!(count, 1);
    assert!(
        store.read(&id).unwrap().is_some(),
        "memory is findable after reindex"
    );
}
