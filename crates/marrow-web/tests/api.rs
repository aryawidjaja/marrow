//! Exercise the dashboard API by calling `route` directly (no socket needed).

use std::path::Path;

use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::Store;
use marrow_web::route;

fn mem(kind: MemoryKind, topic: &str, body: &str) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(),
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
                written_by: "agent".into(),
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

fn get(root: &Path, target: &str) -> serde_json::Value {
    let resp = route(Some(root), "GET", target, "");
    assert_eq!(resp.status, 200, "GET {target}");
    serde_json::from_str(&resp.body).unwrap()
}

#[test]
fn serves_the_dashboard_html() {
    let dir = tempfile::tempdir().unwrap();
    Store::init(dir.path()).unwrap();
    let resp = route(Some(dir.path()), "GET", "/", "");
    assert_eq!(resp.status, 200);
    assert!(resp.content_type.starts_with("text/html"));
    assert!(resp.body.contains("Marrow"));
    assert!(resp.body.contains("channel-shell"));
    assert!(resp.body.contains("CHEVRON_RIGHT"));
    assert!(resp.body.contains("@media (max-width: 780px)"));
    assert!(resp
        .body
        .contains("#channel .rooms { flex: 1 1 auto; min-height: 0;"));
    assert!(resp
        .body
        .contains("#channel .conversation { min-width: 0; min-height: 0;"));
    assert!(resp
        .body
        .contains("#channel .messages { min-height: 0; overflow-x: hidden; overflow-y: auto;"));
    assert!(resp.body.contains("function linkMatchesFilters(l)"));
    assert!(resp
        .body
        .contains("l.s.by !== byFilter || l.t.by !== byFilter"));
    assert!(resp.body.contains("Review suggestions"));
    assert!(resp.body.contains("Trace to another memory"));
    assert!(resp.body.contains("id=\"graph-sidebar\""));
    assert!(resp.body.contains("data-side-section=\"projects\""));
    assert!(resp.body.contains("data-side-section=\"insights\""));
    assert!(resp.body.contains("id=\"sidebar-toggle\""));
    assert!(resp.body.contains("setMobileSidebar"));
    assert!(resp
        .body
        .contains(".legend { position: static; width: auto;"));
    assert!(!resp.body.contains("<span class=\"caret\">▶</span>"));
}

#[test]
fn reads_one_memory_with_body() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(
        MemoryKind::Decision,
        "storage",
        "Markdown is the source of truth.",
    );
    let id = store.write(&mut a).unwrap();

    let v = get(dir.path(), &format!("/api/memory/{id}"));
    assert_eq!(v["type"], "decision");
    assert_eq!(
        v["body"].as_str().unwrap().trim(),
        "Markdown is the source of truth."
    );

    let missing = route(Some(dir.path()), "GET", "/api/memory/nope", "");
    assert_eq!(missing.status, 404);
}

#[test]
fn audit_reports_the_chain_and_how_much_it_covers() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "x", "a fact");
    store.write(&mut a).unwrap();

    let audit = get(dir.path(), "/api/audit");
    assert_eq!(audit["ok"], true);
    // "chain intact" over an unstated number of events proves nothing to the person reading it.
    assert_eq!(audit["events"], 1);
}

#[test]
fn stale_endpoint_returns_empty_without_anchors() {
    let dir = tempfile::tempdir().unwrap();
    Store::init(dir.path()).unwrap();
    let v = get(dir.path(), "/api/stale");
    assert_eq!(v["count"], 0);
}

#[test]
fn onboarding_reports_the_served_project_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    std::fs::write(dir.path().join("README.md"), "# Demo").unwrap();
    let mut memory = mem(MemoryKind::Fact, "scope", "Only this project.");
    store.write(&mut memory).unwrap();

    let value = get(dir.path(), "/api/onboarding");
    assert_eq!(value["memories"], 1);
    assert_eq!(value["docs"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["root"],
        dir.path().canonicalize().unwrap().display().to_string()
    );
}

#[test]
fn provenance_endpoint_returns_trail() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Decision, "auth", "Use JWT.");
    let old = store.write(&mut a).unwrap();
    let mut b = mem(MemoryKind::Decision, "auth", "Use opaque tokens.");
    let new = store.supersede(&old, &mut b).unwrap();

    let v = get(dir.path(), &format!("/api/provenance/{new}"));
    assert_eq!(v["written_by"], "agent");
    assert!(v["supersedes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["id"] == old));
    assert!(!v["history"].as_array().unwrap().is_empty());
}

#[test]
fn create_edit_delete_memory_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    Store::init(dir.path()).unwrap();

    // Create via the dashboard.
    let created = route(
        Some(dir.path()),
        "POST",
        "/api/memory",
        r#"{"kind":"fact","topic":"cache","area":"storage","body":"Cache clears on write.","tags":["perf"],"confidence":0.8,"expires_at":"2027-01-01T00:00:00Z","sources":["docs/cache.md"]}"#,
    );
    let cv: serde_json::Value = serde_json::from_str(&created.body).unwrap();
    assert_eq!(cv["ok"], true);
    let id = cv["id"].as_str().unwrap().to_string();
    let created_memory = get(dir.path(), &format!("/api/memory/{id}"));
    assert_eq!(created_memory["area"], "storage");
    assert_eq!(created_memory["confidence"], 0.8);
    assert_eq!(
        created_memory["decay"]["expires_at"],
        "2027-01-01T00:00:00Z"
    );
    assert_eq!(created_memory["provenance"]["sources"][0], "docs/cache.md");

    // Edit it.
    let edited = route(
        Some(dir.path()),
        "POST",
        &format!("/api/memory/{id}/edit"),
        r#"{"body":"Cache clears on every write, always."}"#,
    );
    assert_eq!(edited.status, 200);
    let v = get(dir.path(), &format!("/api/memory/{id}"));
    assert!(v["body"].as_str().unwrap().contains("always"));

    // Delete it.
    let del = route(
        Some(dir.path()),
        "POST",
        &format!("/api/memory/{id}/delete"),
        "",
    );
    assert_eq!(del.status, 200);
    let gone = route(Some(dir.path()), "GET", &format!("/api/memory/{id}"), "");
    assert_eq!(gone.status, 404);
}

#[test]
fn graph_api_exposes_trust_review_and_explainable_paths() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "shared", "first");
    let mut b = mem(MemoryKind::Fact, "shared", "second");
    let a_id = store.write(&mut a).unwrap();
    let b_id = store.write(&mut b).unwrap();

    let graph = get(dir.path(), "/api/graph");
    assert_eq!(graph["links"][0]["trust"], "inferred");
    assert!(graph["links"][0]["confidence"].as_f64().unwrap() > 0.0);
    assert!(!graph["nodes"][0]["community"].as_str().unwrap().is_empty());
    assert!(graph["review"].is_array());
    assert!(graph["review"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["action"]
            .as_str()
            .is_some_and(|action| !action.is_empty())));

    let path = get(
        dir.path(),
        &format!("/api/path?source={a_id}&target={b_id}"),
    );
    assert_eq!(path["found"], true);
    assert_eq!(path["links"][0]["rel"], "topic");
    assert_eq!(path["links"][0]["trust"], "inferred");
}
