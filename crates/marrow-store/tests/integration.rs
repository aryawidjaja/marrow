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
            area: None,
            scope: Scope {
                project_id: "demo".into(),
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: "agent-1".into(),
                model: None,
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

/// A deterministic embedder with a tiny synonym map, so semantic search can be tested
/// independently of keyword overlap. "red"/"crimson" -> [1,0,0]; "green" -> [0,1,0];
/// anything else -> [0,0,1].
struct StubEmbedder;

impl marrow_store::Embedder for StubEmbedder {
    fn dim(&self) -> usize {
        3
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, marrow_store::EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let t = t.to_lowercase();
                if t.contains("red") || t.contains("crimson") {
                    vec![1.0, 0.0, 0.0]
                } else if t.contains("green") {
                    vec![0.0, 1.0, 0.0]
                } else {
                    vec![0.0, 0.0, 1.0]
                }
            })
            .collect())
    }
}

#[test]
fn semantic_search_surfaces_a_keyword_miss() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::init(dir.path()).unwrap();
    store.set_embedder(Box::new(StubEmbedder));

    let mut a = mem(
        MemoryKind::Fact,
        "alerts",
        "RED alert when the disk fills up",
    );
    let id = store.write(&mut a).unwrap();
    let mut b = mem(
        MemoryKind::Fact,
        "fields",
        "GREEN pastures and rolling hills",
    );
    store.write(&mut b).unwrap();

    // "crimson" shares no term with any body, so keyword search finds nothing...
    let kw = Query {
        project_id: Some("demo".into()),
        hybrid_weight: Some(0.0),
        ..Query::default()
    };
    assert!(store.search("crimson", &kw).unwrap().is_empty());

    // ...but semantically it maps to the RED memory, and only that one.
    let sem = Query {
        project_id: Some("demo".into()),
        hybrid_weight: Some(1.0),
        ..Query::default()
    };
    let hits = store.search("crimson", &sem).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].frontmatter.id, id);
}

#[test]
fn weight_zero_is_keyword_only_even_with_embedder() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::init(dir.path()).unwrap();
    store.set_embedder(Box::new(StubEmbedder));
    let mut a = mem(MemoryKind::Fact, "x", "alpha beta gamma");
    store.write(&mut a).unwrap();

    let kw = Query {
        project_id: Some("demo".into()),
        hybrid_weight: Some(0.0),
        ..Query::default()
    };
    assert_eq!(store.search("alpha", &kw).unwrap().len(), 1);
    // A semantic-only synonym query returns nothing at w=0.
    assert!(store.search("crimson", &kw).unwrap().is_empty());
}

#[test]
fn reembed_backfills_vectors() {
    let dir = tempfile::tempdir().unwrap();
    // Written with no embedder configured -> no vectors stored.
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "zone", "the RED zone is off limits");
    store.write(&mut m).unwrap();

    // Reopen, attach an embedder, and backfill.
    let mut store = Store::open(dir.path()).unwrap();
    store.set_embedder(Box::new(StubEmbedder));
    assert!(store.reembed().unwrap() >= 1);

    let sem = Query {
        project_id: Some("demo".into()),
        hybrid_weight: Some(1.0),
        ..Query::default()
    };
    assert_eq!(store.search("crimson", &sem).unwrap().len(), 1);
}

#[test]
fn writes_are_recorded_in_the_audit_log_and_chain_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "auth", "rotate keys every 90 days");
    let id = store.write(&mut a).unwrap();

    let history = store.history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, "write");
    assert_eq!(history[0].memory_id.as_deref(), Some(id.as_str()));
    assert_eq!(store.verify_log(), Ok(()));
}

#[test]
fn supersede_records_write_and_supersede_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut old = mem(MemoryKind::Decision, "auth", "use JWT");
    let old_id = store.write(&mut old).unwrap();
    let mut new = mem(MemoryKind::Decision, "auth", "use opaque tokens");
    store.supersede(&old_id, &mut new).unwrap();

    let kinds: Vec<String> = store
        .history()
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    // write(old) + write(old as superseded) + write(new) + supersede
    assert!(kinds.contains(&"supersede".to_string()));
    assert!(kinds.iter().filter(|k| *k == "write").count() >= 2);
    assert_eq!(store.verify_log(), Ok(()));
}

#[test]
fn tampering_with_the_log_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "auth", "a sensitive fact");
    store.write(&mut a).unwrap(); // recorded actor is "agent-1"

    // Rewrite history: swap the recorded actor. The hash chain must catch it.
    let log_path = dir.path().join(".marrow/episodic/log.jsonl");
    let content = std::fs::read_to_string(&log_path).unwrap();
    std::fs::write(&log_path, content.replace("agent-1", "intruder")).unwrap();

    assert!(store.verify_log().is_err());
}

#[test]
fn consolidate_detects_without_changing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "a", "the disk is full");
    let a_id = store.write(&mut a).unwrap();
    let mut b = mem(MemoryKind::Fact, "b", "the disk is full");
    store.write(&mut b).unwrap();

    let report = store.consolidate(dir.path()).unwrap();
    assert_eq!(report.clusters.len(), 1);
    assert_eq!(report.clusters[0].others.len(), 1);
    // Read-only: both still active.
    assert_eq!(
        store.read(&a_id).unwrap().unwrap().frontmatter.status,
        Status::Active
    );
}

#[test]
fn consolidate_apply_merges_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "a", "the disk is full");
    let a_id = store.write(&mut a).unwrap();
    let mut b = mem(MemoryKind::Fact, "b", "the disk is full");
    let b_id = store.write(&mut b).unwrap();

    let outcome = store.consolidate_apply(dir.path()).unwrap();
    assert_eq!(outcome.merged, 1);

    // The newer one (b) is kept active and now supersedes the older (a).
    let kept = store.read(&b_id).unwrap().unwrap();
    assert_eq!(kept.frontmatter.status, Status::Active);
    assert!(kept.frontmatter.supersedes.contains(&a_id));
    assert_eq!(
        store.read(&a_id).unwrap().unwrap().frontmatter.status,
        Status::Superseded
    );

    // Only one active fact with that body remains.
    let active = store
        .query(&Query {
            kind: Some(MemoryKind::Fact),
            status: Some(Status::Active),
            project_id: Some("demo".into()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(active.len(), 1);
}

#[test]
fn consolidate_apply_retires_expired() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "temp", "ephemeral note");
    m.frontmatter.decay = Some(Decay {
        expires_at: Some("2000-01-01T00:00:00Z".into()),
    });
    let id = store.write(&mut m).unwrap();

    let outcome = store.consolidate_apply(dir.path()).unwrap();
    assert_eq!(outcome.deprecated, 1);
    assert_eq!(
        store.read(&id).unwrap().unwrap().frontmatter.status,
        Status::Deprecated
    );
}

/// A distiller that always reports a conflict, with a fixed resolved body.
struct ConflictDistiller;
impl marrow_store::Distiller for ConflictDistiller {
    fn distill(&self, _bodies: &[String]) -> Result<marrow_store::Verdict, String> {
        Ok(marrow_store::Verdict {
            action: marrow_store::ClusterAction::Conflict,
            body: "resolved: the cache is invalidated on write".to_string(),
            rationale: "kept the higher-confidence statement".to_string(),
        })
    }
}

#[test]
fn semantic_clustering_groups_differently_worded_memories() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::init(dir.path()).unwrap();
    store.set_embedder(Box::new(StubEmbedder)); // "red" -> same vector regardless of wording

    // Same meaning, different words -> exact-match would miss this; semantic catches it.
    let mut a = mem(
        MemoryKind::Fact,
        "alerts",
        "a RED alert fires when the disk fills",
    );
    store.write(&mut a).unwrap();
    let mut b = mem(
        MemoryKind::Fact,
        "alerts",
        "disk exhaustion raises a RED warning",
    );
    store.write(&mut b).unwrap();

    let report = store.consolidate(dir.path()).unwrap();
    assert_eq!(
        report.clusters.len(),
        1,
        "differently-worded memories should cluster"
    );
    assert_eq!(report.clusters[0].others.len(), 1);
}

#[test]
fn salience_keeps_the_higher_confidence_memory() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap(); // no embedder -> exact-match cluster
    let mut weak = mem(MemoryKind::Fact, "a", "the build runs on CI");
    weak.frontmatter.confidence = 0.4;
    let weak_id = store.write(&mut weak).unwrap();
    let mut strong = mem(MemoryKind::Fact, "b", "the build runs on CI");
    strong.frontmatter.confidence = 0.95;
    let strong_id = store.write(&mut strong).unwrap();

    store.consolidate_apply(dir.path()).unwrap();
    // The higher-confidence memory survives; the weaker one is superseded.
    assert_eq!(
        store.read(&strong_id).unwrap().unwrap().frontmatter.status,
        Status::Active
    );
    assert_eq!(
        store.read(&weak_id).unwrap().unwrap().frontmatter.status,
        Status::Superseded
    );
}

#[test]
fn conflict_resolution_records_an_audit_event() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::init(dir.path()).unwrap();
    store.set_distiller(Box::new(ConflictDistiller));
    // Two exact-duplicate bodies so they cluster without an embedder.
    let mut a = mem(MemoryKind::Fact, "cache", "cache cleared on write");
    store.write(&mut a).unwrap();
    let mut b = mem(MemoryKind::Fact, "cache", "cache cleared on write");
    store.write(&mut b).unwrap();

    let outcome = store.consolidate_apply(dir.path()).unwrap();
    assert_eq!(outcome.conflicts_resolved, 1);
    let kinds: Vec<String> = store
        .history()
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"conflict_resolved".to_string()));
    assert_eq!(store.verify_log(), Ok(()));
}

#[test]
fn provenance_traces_lineage_and_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut old = mem(MemoryKind::Decision, "auth", "Use JWT.");
    let old_id = store.write(&mut old).unwrap();
    let mut new = mem(MemoryKind::Decision, "auth", "Use opaque tokens.");
    let new_id = store.supersede(&old_id, &mut new).unwrap();

    let new_trail = store.provenance(&new_id).unwrap().unwrap();
    assert!(new_trail.supersedes.iter().any(|r| r.id == old_id));
    assert_eq!(new_trail.written_by, "agent-1");
    assert!(!new_trail.events.is_empty());

    let old_trail = store.provenance(&old_id).unwrap().unwrap();
    assert!(old_trail.superseded_by.iter().any(|r| r.id == new_id));
}

#[test]
fn recall_logs_a_traceable_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "limits", "rate limit is 100 per minute");
    let id = store.write(&mut m).unwrap();

    let q = Query::for_project("demo");
    let hits = store.recall("rate limit", &q, "agent-7").unwrap();
    assert_eq!(hits.len(), 1);

    // The retrieval is recorded, and shows up in the memory's provenance trail.
    let kinds: Vec<String> = store
        .history()
        .unwrap()
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(kinds.contains(&"retrieve".to_string()));
    let trail = store.provenance(&id).unwrap().unwrap();
    assert!(trail.events.iter().any(|e| e.kind == "retrieve"));
}

#[test]
fn search_matches_word_stems() {
    // A user searching the singular "JWT" should find a memory that stored the plural "JWTs".
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(
        MemoryKind::Fact,
        "auth",
        "We use short-lived JWTs for sessions.",
    );
    store.write(&mut m).unwrap();

    let hits = store.search("JWT", &Query::for_project("demo")).unwrap();
    assert_eq!(
        hits.len(),
        1,
        "singular query must match the plural in the body (porter stemming)"
    );

    // And the reverse: a plural query matches a singular body word.
    let hits2 = store
        .search("session", &Query::for_project("demo"))
        .unwrap();
    assert_eq!(
        hits2.len(),
        1,
        "stemming should match 'session' against 'sessions'"
    );
}

#[test]
fn search_survives_missing_tokens_and_punctuation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut m = mem(
        MemoryKind::Fact,
        "ui",
        "The legend renders a colored symbol.",
    );
    store.write(&mut m).unwrap();
    let q = Query::for_project("demo");

    // A token present in no memory ("autocount") must not AND-collapse the whole query to zero.
    assert_eq!(
        store.search("legend autocount symbol", &q).unwrap().len(),
        1
    );
    // Punctuation that FTS5 reads as operators must not error.
    assert!(store.search("symbol.", &q).is_ok());
    assert!(store.search("E201:", &q).is_ok());
    assert!(store.search("…", &q).unwrap().is_empty());
}

#[test]
fn open_discovers_store_in_an_ancestor_dir() {
    let dir = tempfile::tempdir().unwrap();
    let root = Store::init(dir.path()).unwrap();
    let mut m = mem(MemoryKind::Fact, "x", "shared brain entry");
    root.write(&mut m).unwrap();

    let sub = dir.path().join("nested/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let sub_store = Store::open(&sub).unwrap();
    assert_eq!(sub_store.list().unwrap().len(), 1);
}

/// A memory's file lives under a directory named for its kind, so changing the kind moves it. If the
/// copy at the old path survived, the same id would exist twice on disk and a reindex could revive
/// the retired version over the live one.
#[test]
fn changing_a_memorys_kind_leaves_no_copy_at_the_old_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();

    let mut m = mem(MemoryKind::Entity, "jwt-expiry", "JWTs last 15 minutes.");
    let id = store.write(&mut m).unwrap();
    assert!(dir
        .path()
        .join(".marrow/memory/entity")
        .join(format!("{id}.md"))
        .exists());

    let mut moved = store.read(&id).unwrap().unwrap();
    moved.frontmatter.kind = MemoryKind::Fact;
    store.write(&mut moved).unwrap();

    assert!(
        !dir.path()
            .join(".marrow/memory/entity")
            .join(format!("{id}.md"))
            .exists(),
        "the file at the old kind's path must not survive the move"
    );
    assert!(dir
        .path()
        .join(".marrow/memory/fact")
        .join(format!("{id}.md"))
        .exists());

    // And a reindex must not resurrect anything from a stranded file.
    store.reindex().unwrap();
    let rows: Vec<_> = store
        .list()
        .unwrap()
        .into_iter()
        .filter(|r| r.id == id)
        .collect();
    assert_eq!(rows.len(), 1, "one id, one row");
    assert!(rows[0].path.starts_with("fact/"), "got {}", rows[0].path);
}
