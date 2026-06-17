//! A local HTTP dashboard for a Marrow store.
//!
//! [`route`] turns a request into a response over an open [`Store`]; it is a plain function so
//! the whole API is testable without binding a socket. [`serve`] runs the tiny_http loop.

use std::collections::HashMap;
use std::path::Path;

use marrow_store::Store;
use serde_json::json;

/// The dashboard single-page app, embedded so the binary is self-contained.
pub const DASHBOARD: &str = include_str!("../assets/dashboard.html");

/// A ready-to-send HTTP response.
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

/// Route one request against the store. `root` is the project the store lives in (used for
/// staleness and consolidation, which look at the live code).
pub fn route(store: &Store, root: &Path, method: &str, target: &str) -> Response {
    let (path, query) = split_target(target);
    match (method, path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => html(DASHBOARD),
        ("GET", "/api/memories") => memories(store, &query),
        ("GET", "/api/stale") => stale(store, root),
        ("GET", "/api/history") => history(store),
        ("GET", "/api/audit") => audit(store),
        ("POST", "/api/consolidate") => consolidate(store, root, &query),
        ("GET", p) if p.starts_with("/api/memory/") => {
            memory(store, p.trim_start_matches("/api/memory/"))
        }
        _ => not_found(),
    }
}

/// Run the dashboard server until the process is killed.
pub fn serve(root: &Path, addr: &str) -> Result<(), String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    let server = tiny_http::Server::http(addr).map_err(|e| e.to_string())?;
    println!(
        "Marrow dashboard on http://{addr}  (store: {})",
        root.display()
    );
    for request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let target = request.url().to_string();
        let resp = route(&store, root, &method, &target);
        let header =
            tiny_http::Header::from_bytes(&b"Content-Type"[..], resp.content_type.as_bytes())
                .unwrap();
        let http = tiny_http::Response::from_string(resp.body)
            .with_status_code(resp.status)
            .with_header(header);
        let _ = request.respond(http);
    }
    Ok(())
}

fn memories(store: &Store, query: &HashMap<String, String>) -> Response {
    let want_status = query.get("status").map(String::as_str).unwrap_or("active");
    let want_kind = query.get("kind").map(String::as_str);
    let rows = match store.list() {
        Ok(r) => r,
        Err(e) => return error(&e.to_string()),
    };
    let items: Vec<_> = rows
        .into_iter()
        .filter(|r| want_status == "all" || r.status == want_status)
        .filter(|r| want_kind.is_none_or(|k| r.kind == k))
        .map(|r| {
            json!({
                "id": r.id,
                "kind": r.kind,
                "topic": r.topic,
                "status": r.status,
                "confidence": r.confidence,
                "updated_at": r.updated_at,
                "snippet": r.body.lines().next().unwrap_or("").trim(),
            })
        })
        .collect();
    json_ok(json!({ "memories": items, "count": items.len() }))
}

fn memory(store: &Store, id: &str) -> Response {
    match store.read(id) {
        Ok(Some(m)) => {
            let mut v = serde_json::to_value(&m.frontmatter).unwrap_or_else(|_| json!({}));
            v["body"] = json!(m.body);
            json_ok(v)
        }
        Ok(None) => not_found(),
        Err(e) => error(&e.to_string()),
    }
}

fn stale(store: &Store, root: &Path) -> Response {
    match store.list_stale(root) {
        Ok(hits) => {
            let items: Vec<_> = hits
                .iter()
                .map(|h| {
                    json!({
                        "memory_id": h.memory_id,
                        "symbol": h.symbol,
                        "file_path": h.file_path,
                        "relocated_to": h.relocated_to,
                    })
                })
                .collect();
            json_ok(json!({ "stale": items, "count": items.len() }))
        }
        Err(e) => error(&e.to_string()),
    }
}

fn history(store: &Store) -> Response {
    match store.history() {
        Ok(events) => json_ok(json!({
            "events": serde_json::to_value(&events).unwrap_or_else(|_| json!([])),
            "count": events.len(),
        })),
        Err(e) => error(&e.to_string()),
    }
}

fn audit(store: &Store) -> Response {
    match store.verify_log() {
        Ok(()) => json_ok(json!({ "ok": true })),
        Err(seq) => json_ok(json!({ "ok": false, "broken_at_seq": seq })),
    }
}

fn consolidate(store: &Store, root: &Path, query: &HashMap<String, String>) -> Response {
    let apply = query.get("apply").map(String::as_str) == Some("true");
    if apply {
        match store.consolidate_apply(root) {
            Ok(o) => json_ok(json!({
                "applied": true,
                "deprecated": o.deprecated,
                "merged": o.merged,
                "conflicts_resolved": o.conflicts_resolved,
            })),
            Err(e) => error(&e.to_string()),
        }
    } else {
        match store.consolidate(root) {
            Ok(r) => {
                let related: usize = r.clusters.iter().map(|c| c.others.len()).sum();
                json_ok(json!({
                    "stale": r.stale.len(),
                    "expired": r.expired.len(),
                    "related_memories": related,
                    "clusters": r.clusters.len(),
                }))
            }
            Err(e) => error(&e.to_string()),
        }
    }
}

fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let mut parts = target.splitn(2, '?');
    let path = parts.next().unwrap_or("/").to_string();
    let mut query = HashMap::new();
    if let Some(qs) = parts.next() {
        for pair in qs.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                query.insert(k.to_string(), v.to_string());
            }
        }
    }
    (path, query)
}

fn html(body: &str) -> Response {
    Response {
        status: 200,
        content_type: "text/html; charset=utf-8",
        body: body.to_string(),
    }
}

fn json_ok(value: serde_json::Value) -> Response {
    Response {
        status: 200,
        content_type: "application/json",
        body: value.to_string(),
    }
}

fn not_found() -> Response {
    Response {
        status: 404,
        content_type: "application/json",
        body: json!({ "error": "not found" }).to_string(),
    }
}

fn error(message: &str) -> Response {
    Response {
        status: 500,
        content_type: "application/json",
        body: json!({ "error": message }).to_string(),
    }
}
