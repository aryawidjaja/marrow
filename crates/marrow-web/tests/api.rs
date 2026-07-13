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
