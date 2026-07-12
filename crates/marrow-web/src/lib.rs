//! A local HTTP dashboard for a Marrow store.
//!
//! [`route`] turns a request into a response over an open [`Store`]; it is a plain function so
//! the whole API is testable without binding a socket. [`serve`] runs the tiny_http loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::{Hub, Store};
use serde_json::{json, Value};

mod graph;

/// The dashboard single-page app, embedded so the binary is self-contained.
pub const DASHBOARD: &str = include_str!("../assets/dashboard.html");

/// A ready-to-send HTTP response.
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

/// Route one request. The dashboard is hub-aware: hive and project-management endpoints need no
/// store, and project-scoped endpoints resolve their store from `?project=<name>` (falling back to
/// `default_root`, the store `marrow-serve --root` was started on, if any). `body` is the request
/// body for POSTs.
pub fn route(default_root: Option<&Path>, method: &str, target: &str, body: &str) -> Response {
    let (path, query) = split_target(target);
    let p = path.as_str();

    // Global endpoints — no single project store.
    match (method, p) {
        ("GET", "/") | ("GET", "/index.html") => return html(DASHBOARD),
        ("GET", "/api/hive") => return graph_hive(),
        ("GET", "/api/hive/memory") => return hive_memory(&query),
        ("GET", "/api/projects") => return projects_list(default_root),
        ("GET", "/api/channel") => return channel(default_root),
        ("GET", "/api/browse") => return browse(&query),
        ("POST", "/api/project/register") => return hub_register(body),
        ("POST", "/api/project/forget") => return hub_forget(body),
        ("POST", "/api/project/init") => return hub_init(body),
        _ => {}
    }

    // Project-scoped endpoints.
    let Some(root) = project_root(default_root, query.get("project").map(String::as_str)) else {
        return error("no project — pass ?project=<name> or start marrow-serve with --root");
    };
    let store = match Store::open(&root) {
        Ok(s) => s,
        Err(e) => return error(&e.to_string()),
    };
    match (method, p) {
        ("GET", "/api/graph") => graph_project(&store, &root),
        ("GET", "/api/search") => search_memories(&store, &query),
        ("GET", "/api/memories") => memories(&store, &query),
        ("GET", "/api/stale") => stale(&store, &root),
        ("GET", "/api/history") => history(&store),
        ("GET", "/api/audit") => audit(&store),
        ("GET", "/api/evidence") => evidence(&store, &root),
        ("POST", "/api/memory") => create_memory(&store, body),
        ("POST", "/api/link") => link(&root, &query, true),
        ("POST", "/api/unlink") => link(&root, &query, false),
        ("POST", "/api/hide") => hide(&root, &query, true),
        ("POST", "/api/unhide") => hide(&root, &query, false),
        ("POST", "/api/consolidate") => consolidate(&store, &root, &query),
        ("POST", "/api/demo/seed") => demo_seed(&store, &root),
        ("POST", "/api/demo/break") => demo_break(&root),
        ("POST", pp) if pp.starts_with("/api/memory/") && pp.ends_with("/edit") => {
            edit_memory(&store, mem_id(pp, "/edit"), body)
        }
        ("POST", pp) if pp.starts_with("/api/memory/") && pp.ends_with("/delete") => {
            delete_memory(&store, mem_id(pp, "/delete"))
        }
        ("GET", pp) if pp.starts_with("/api/memory/") => {
            memory(&store, pp.trim_start_matches("/api/memory/"))
        }
        ("GET", pp) if pp.starts_with("/api/provenance/") => {
            provenance(&store, pp.trim_start_matches("/api/provenance/"))
        }
        _ => not_found(),
    }
}

/// Resolve which store a request targets: a named registered project, the shared `core`, or the
/// store the server was started on.
fn project_root(default_root: Option<&Path>, project: Option<&str>) -> Option<PathBuf> {
    match project {
        Some("core") => Hub::open()
            .ok()
            .and_then(|h| h.core().ok())
            .map(|s| s.root().to_path_buf()),
        Some(name) if !name.is_empty() => Hub::open()
            .ok()
            .and_then(|h| h.projects().into_iter().find(|p| p.name == name))
            .map(|p| p.root),
        _ => default_root.map(Path::to_path_buf),
    }
}

fn mem_id<'a>(path: &'a str, suffix: &str) -> &'a str {
    path.trim_start_matches("/api/memory/")
        .trim_end_matches(suffix)
}

/// The projects the switcher offers: every registered project, plus the served store when it isn't
/// itself registered.
fn projects_list(default_root: Option<&Path>) -> Response {
    let mut items = Vec::new();
    let registered = Hub::open().map(|h| h.projects()).unwrap_or_default();
    if let Some(r) = default_root {
        let canon = r.canonicalize().unwrap_or_else(|_| r.to_path_buf());
        if !registered.iter().any(|p| p.root == canon) {
            items.push(
                json!({"name": "· this project", "project": "", "root": r.display().to_string()}),
            );
        }
    }
    for p in &registered {
        items
            .push(json!({"name": p.name, "project": p.name, "root": p.root.display().to_string()}));
    }
    json_ok(json!({ "projects": items }))
}

/// List sub-directories of a path so the UI can navigate the filesystem to add a project. Local
/// dashboard only, so browsing the user's own machine is fine.
fn browse(query: &HashMap<String, String>) -> Response {
    let path = query
        .get("path")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(home_dir);
    let mut dirs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&path) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !e.path().is_dir() {
                continue;
            }
            dirs.push(json!({
                "name": name,
                "path": e.path().display().to_string(),
                "marrow": e.path().join(".marrow").is_dir(),
            }));
        }
    }
    dirs.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    json_ok(json!({
        "path": path.display().to_string(),
        "parent": path.parent().map(|p| p.display().to_string()),
        "dirs": dirs,
    }))
}

/// The agent channel: conversation threads from the shared `core` bus (or the served store if
/// there's no hive), for the dashboard's Channel view.
fn channel(default_root: Option<&Path>) -> Response {
    let store = if Hub::active() {
        Hub::open().and_then(|h| h.core())
    } else if let Some(r) = default_root {
        Store::open(r)
    } else {
        return json_ok(json!({ "threads": [] }));
    };
    let store = match store {
        Ok(s) => s,
        Err(e) => return error(&e.to_string()),
    };
    match store.channel_threads(50) {
        Ok(threads) => {
            let items: Vec<Value> = threads
                .iter()
                .filter_map(|ms| {
                    ms.first().map(|first| {
                        json!({
                            "thread": first.thread,
                            "messages": ms.iter().map(|m| json!({
                                "from": m.from, "to": m.to, "role": m.role, "body": m.body, "ts": m.ts
                            })).collect::<Vec<_>>(),
                        })
                    })
                })
                .collect();
            json_ok(json!({ "threads": items }))
        }
        Err(e) => error(&e.to_string()),
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn hub_register(body: &str) -> Response {
    let v = parse_body(body);
    let Some(path) = v.get("path").and_then(Value::as_str) else {
        return error("register needs a path");
    };
    let name = v.get("name").and_then(Value::as_str);
    match Hub::open().and_then(|mut h| h.register(Path::new(path), name)) {
        Ok(p) => json_ok(json!({ "ok": true, "name": p.name })),
        Err(e) => error(&e.to_string()),
    }
}

fn hub_forget(body: &str) -> Response {
    let v = parse_body(body);
    let Some(key) = v.get("name").and_then(Value::as_str) else {
        return error("forget needs a name");
    };
    match Hub::open().and_then(|mut h| h.forget(key)) {
        Ok(removed) => json_ok(json!({ "ok": removed })),
        Err(e) => error(&e.to_string()),
    }
}

fn hub_init(body: &str) -> Response {
    let v = parse_body(body);
    let Some(path) = v.get("path").and_then(Value::as_str) else {
        return error("init needs a path");
    };
    let name = v.get("name").and_then(Value::as_str);
    if let Err(e) = Store::init(path) {
        return error(&e.to_string());
    }
    match Hub::open().and_then(|mut h| h.register(Path::new(path), name)) {
        Ok(p) => json_ok(json!({ "ok": true, "name": p.name })),
        Err(e) => error(&e.to_string()),
    }
}

fn create_memory(store: &Store, body: &str) -> Response {
    let v = parse_body(body);
    let kind = match v.get("kind").and_then(Value::as_str).map(parse_kind) {
        Some(Ok(k)) => k,
        _ => return error("memory needs a valid kind (fact|decision|entity|session|skill)"),
    };
    let Some(text) = v.get("body").and_then(Value::as_str) else {
        return error("memory needs a body");
    };
    let mut memory = new_memory(
        kind,
        v.get("topic").and_then(Value::as_str),
        text,
        str_list(&v, "tags"),
    );
    match store.write(&mut memory) {
        Ok(id) => json_ok(json!({ "ok": true, "id": id })),
        Err(e) => error(&e.to_string()),
    }
}

fn edit_memory(store: &Store, id: &str, body: &str) -> Response {
    let v = parse_body(body);
    let topic = v.get("topic").and_then(Value::as_str).map(String::from);
    let text = v.get("body").and_then(Value::as_str).map(String::from);
    let tags = v.get("tags").map(|_| str_list(&v, "tags"));
    match store.update(id, topic, text, tags) {
        Ok(true) => json_ok(json!({ "ok": true })),
        Ok(false) => not_found(),
        Err(e) => error(&e.to_string()),
    }
}

fn delete_memory(store: &Store, id: &str) -> Response {
    match store.delete(id) {
        Ok(true) => json_ok(json!({ "ok": true })),
        Ok(false) => not_found(),
        Err(e) => error(&e.to_string()),
    }
}

fn search_memories(store: &Store, query: &HashMap<String, String>) -> Response {
    let q = query.get("q").map(String::as_str).unwrap_or_default();
    let found = store
        .search(
            q,
            &marrow_store::Query {
                limit: Some(30),
                ..Default::default()
            },
        )
        .unwrap_or_default();
    let items: Vec<Value> = found
        .iter()
        .map(|m| {
            json!({
                "id": m.frontmatter.id,
                "kind": kind_str(m.frontmatter.kind),
                "topic": m.frontmatter.topic,
                "snippet": m.body.lines().next().unwrap_or("").trim(),
            })
        })
        .collect();
    json_ok(json!({ "results": items, "count": items.len() }))
}

fn parse_body(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| json!({}))
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_kind(s: &str) -> Result<MemoryKind, ()> {
    match s {
        "fact" => Ok(MemoryKind::Fact),
        "decision" => Ok(MemoryKind::Decision),
        "entity" => Ok(MemoryKind::Entity),
        "session" => Ok(MemoryKind::Session),
        "skill" => Ok(MemoryKind::Skill),
        _ => Err(()),
    }
}

fn kind_str(k: MemoryKind) -> &'static str {
    match k {
        MemoryKind::Fact => "fact",
        MemoryKind::Decision => "decision",
        MemoryKind::Entity => "entity",
        MemoryKind::Session => "session",
        MemoryKind::Skill => "skill",
    }
}

fn new_memory(kind: MemoryKind, topic: Option<&str>, body: &str, tags: Vec<String>) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(),
            kind,
            status: Status::Active,
            topic: topic.filter(|t| !t.is_empty()).map(String::from),
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: String::new(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: "web".into(),
                session_id: None,
                sources: vec![],
            },
            supersedes: vec![],
            tags,
            created_at: String::new(),
            updated_at: String::new(),
            hmac: None,
        },
        body: body.into(),
    }
}

fn graph_project(store: &Store, root: &Path) -> Response {
    Response {
        status: 200,
        content_type: "application/json",
        body: graph::to_json(&graph::project_graph(store, root)),
    }
}

fn graph_hive() -> Response {
    match Hub::open() {
        Ok(hub) => Response {
            status: 200,
            content_type: "application/json",
            body: graph::to_json(&graph::hive_graph(&hub)),
        },
        Err(e) => error(&e.to_string()),
    }
}

/// Read one memory in full from any registered project (or the shared core), so the hive view can
/// show a whole memory that lives in a different store than the one this dashboard was opened on.
fn hive_memory(query: &HashMap<String, String>) -> Response {
    let (Some(project), Some(id)) = (query.get("project"), query.get("id")) else {
        return error("hive memory needs project and id");
    };
    let hub = match Hub::open() {
        Ok(h) => h,
        Err(e) => return error(&e.to_string()),
    };
    let root = if project == "core" {
        hub.core().ok().map(|s| s.root().to_path_buf())
    } else {
        hub.projects()
            .into_iter()
            .find(|p| &p.name == project)
            .map(|p| p.root)
    };
    match root.and_then(|r| Store::open(&r).ok()) {
        Some(store) => memory(&store, id),
        None => not_found(),
    }
}

fn link(root: &Path, query: &HashMap<String, String>, add: bool) -> Response {
    let (Some(source), Some(target)) = (query.get("source"), query.get("target")) else {
        return error("link needs source and target");
    };
    match graph::edit_link(root, source, target, add) {
        Ok(count) => json_ok(json!({ "ok": true, "links": count })),
        Err(e) => error(&e.to_string()),
    }
}

fn hide(root: &Path, query: &HashMap<String, String>, hide: bool) -> Response {
    let (Some(source), Some(target)) = (query.get("source"), query.get("target")) else {
        return error("hide needs source and target");
    };
    match graph::edit_hidden(root, source, target, hide) {
        Ok(count) => json_ok(json!({ "ok": true, "hidden": count })),
        Err(e) => error(&e.to_string()),
    }
}

fn provenance(store: &Store, id: &str) -> Response {
    match store.provenance(id) {
        Ok(Some(t)) => {
            let mref = |r: &marrow_store::MemoryRef| json!({"id": r.id, "kind": r.kind, "topic": r.topic, "status": r.status});
            json_ok(json!({
                "id": t.id,
                "written_by": t.written_by,
                "sources": t.sources,
                "supersedes": t.supersedes.iter().map(mref).collect::<Vec<_>>(),
                "superseded_by": t.superseded_by.iter().map(mref).collect::<Vec<_>>(),
                "history": t.events.iter().map(|e| json!({
                    "seq": e.seq, "ts": e.ts, "kind": e.kind, "summary": e.summary
                })).collect::<Vec<_>>(),
            }))
        }
        Ok(None) => not_found(),
        Err(e) => error(&e.to_string()),
    }
}

/// The sample file the demo controls operate on. Only this file is ever touched.
const DEMO_FILE: &str = "marrow_demo.rs";

fn demo_memory(kind: MemoryKind, topic: &str, body: &str) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(),
            kind,
            status: Status::Active,
            topic: Some(topic.into()),
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: String::new(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: "demo".into(),
                session_id: None,
                sources: vec!["demo-seed".into()],
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

/// Seed a self-contained demo: a code file, a memory anchored to it, and a duplicate pair.
fn demo_seed(store: &Store, root: &Path) -> Response {
    let demo_path = root.join(DEMO_FILE);
    let code = "pub fn demo_widget() -> u32 {\n    42\n}\n";
    if let Err(e) = std::fs::write(&demo_path, code) {
        return error(&e.to_string());
    }
    let mut anchored = demo_memory(
        MemoryKind::Decision,
        "demo-widget",
        "The demo widget returns 42 from demo_widget().",
    );
    if let Err(e) = store.write_anchored(root, DEMO_FILE, "demo_widget", &mut anchored) {
        return error(&e.to_string());
    }
    for topic in ["demo-cache", "demo-cache-copy"] {
        let mut dup = demo_memory(
            MemoryKind::Fact,
            topic,
            "The demo cache is cleared on every write.",
        );
        if let Err(e) = store.write(&mut dup) {
            return error(&e.to_string());
        }
    }
    json_ok(json!({ "seeded": true }))
}

/// Change the demo code so its anchored memory goes stale.
fn demo_break(root: &Path) -> Response {
    let demo_path = root.join(DEMO_FILE);
    let code = "pub fn demo_widget() -> u32 {\n    let base = compute_base();\n    base * 7\n}\n";
    match std::fs::write(&demo_path, code) {
        Ok(()) => json_ok(json!({ "broke": true })),
        Err(e) => error(&e.to_string()),
    }
}

/// Run the dashboard server until the process is killed. With `root = None` it opens centralized:
/// the hive and any registered project, no single served store.
pub fn serve(root: Option<PathBuf>, addr: &str) -> Result<(), String> {
    let server = tiny_http::Server::http(addr).map_err(|e| e.to_string())?;
    match &root {
        Some(r) => println!(
            "Marrow dashboard on http://{addr}  (store: {})",
            r.display()
        ),
        None => {
            println!("Marrow dashboard on http://{addr}  (centralized — every registered project)")
        }
    }
    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let target = request.url().to_string();
        let mut body = String::new();
        if method == "POST" {
            let mut reader = std::io::Read::take(request.as_reader(), 1 << 20);
            let _ = std::io::Read::read_to_string(&mut reader, &mut body);
        }
        let resp = route(root.as_deref(), &method, &target, &body);
        let ctype =
            tiny_http::Header::from_bytes(&b"Content-Type"[..], resp.content_type.as_bytes())
                .unwrap();
        // Never cache: the dashboard is a single-file app, and a cached copy after an upgrade is a
        // real source of "it's broken after I refreshed" confusion.
        let nocache =
            tiny_http::Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap();
        let http = tiny_http::Response::from_string(resp.body)
            .with_status_code(resp.status)
            .with_header(ctype)
            .with_header(nocache);
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

/// Marrow's validated benchmark numbers (from `cargo run -p marrow-bench` and the staleness
/// spike — see bench/REPORT.md) alongside this store's live stats.
fn evidence(store: &Store, root: &Path) -> Response {
    let rows = store.list().unwrap_or_default();
    let active = rows.iter().filter(|r| r.status == "active").count();
    let stale = store.list_stale(root).map(|h| h.len()).unwrap_or(0);
    let duplicates = store
        .consolidate(root)
        .map(|r| r.clusters.iter().map(|c| c.others.len()).sum::<usize>())
        .unwrap_or(0);
    let audit_ok = store.verify_log().is_ok();
    json_ok(json!({
        "product": {
            "staleeval": { "false_positive_pct": 1.0, "recall_pct": 98.4 },
            "consoleval": { "precision_pct": 100.0, "recall_pct": 100.0, "false_merges": 0 },
            "tokeneval": { "reduction_pct": 82.5 }
        },
        "store": {
            "memories": rows.len(),
            "active": active,
            "stale": stale,
            "duplicate_memories": duplicates,
            "audit_ok": audit_ok
        }
    }))
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
                query.insert(k.to_string(), pct_decode(v));
            }
        }
    }
    (path, query)
}

/// Minimal percent + `+` decoding so query values (a search phrase, a filesystem path) survive.
fn pct_decode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
