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
                written_by: "agent".into(),
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
}

#[test]
fn lists_active_memories_with_snippets() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "auth", "We rotate keys every 90 days.");
    let id = store.write(&mut a).unwrap();

    let v = get(dir.path(), "/api/memories");
    assert_eq!(v["count"], 1);
    assert_eq!(v["memories"][0]["id"], id);
    assert_eq!(v["memories"][0]["topic"], "auth");
    assert_eq!(v["memories"][0]["snippet"], "We rotate keys every 90 days.");
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
fn reports_history_and_audit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "x", "a fact");
    store.write(&mut a).unwrap();

    let hist = get(dir.path(), "/api/history");
    assert_eq!(hist["count"], 1);
    assert_eq!(hist["events"][0]["kind"], "write");

    let audit = get(dir.path(), "/api/audit");
    assert_eq!(audit["ok"], true);
}

#[test]
fn stale_endpoint_returns_empty_without_anchors() {
    let dir = tempfile::tempdir().unwrap();
    Store::init(dir.path()).unwrap();
    let v = get(dir.path(), "/api/stale");
    assert_eq!(v["count"], 0);
}

#[test]
fn consolidate_endpoint_merges_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let mut a = mem(MemoryKind::Fact, "a", "the cache is invalidated on write");
    store.write(&mut a).unwrap();
    let mut b = mem(MemoryKind::Fact, "b", "the cache is invalidated on write");
    store.write(&mut b).unwrap();

    let report = route(Some(dir.path()), "POST", "/api/consolidate", "");
    let rv: serde_json::Value = serde_json::from_str(&report.body).unwrap();
    assert_eq!(rv["related_memories"], 1);

    let applied = route(Some(dir.path()), "POST", "/api/consolidate?apply=true", "");
    let av: serde_json::Value = serde_json::from_str(&applied.body).unwrap();
    assert_eq!(av["merged"], 1);

    // The superseded duplicate drops out of the active list.
    let v = get(dir.path(), "/api/memories?status=active");
    assert_eq!(v["count"], 1);
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
        r#"{"kind":"fact","topic":"cache","body":"Cache clears on write.","tags":["perf"]}"#,
    );
    let cv: serde_json::Value = serde_json::from_str(&created.body).unwrap();
    assert_eq!(cv["ok"], true);
    let id = cv["id"].as_str().unwrap().to_string();

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

    // Search finds it.
    let s = get(dir.path(), "/api/search?q=cache");
    assert!(s["count"].as_u64().unwrap() >= 1);

    // Delete it.
    let del = route(
        Some(dir.path()),
        "POST",
        &format!("/api/memory/{id}/delete"),
        "",
    );
    assert_eq!(del.status, 200);
    assert_eq!(get(dir.path(), "/api/memories")["count"], 0);
}
