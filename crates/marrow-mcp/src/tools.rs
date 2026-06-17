//! Tool definitions and execution against a Marrow store.

use std::path::{Path, PathBuf};

use marrow_core::seed_anchor;
use marrow_memdocs::{
    CodeAnchor, Frontmatter, Memory, MemoryKind, Provenance, Ref, RefKind, Scope, Status,
};
use marrow_store::{Query, Store};
use serde_json::{json, Value};

/// The tool catalog advertised via `tools/list`.
pub fn definitions() -> Value {
    json!([
        tool("mem_write", "Write a new memory (fact, decision, entity, session, or skill). Rejects invalid writes with the reasons.", json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
                "body": {"type": "string"},
                "topic": {"type": "string"},
                "project": {"type": "string"},
                "by": {"type": "string", "description": "author for provenance"},
                "tags": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["kind","body"]
        })),
        tool("mem_anchor", "Write a memory anchored to a code symbol so it can be checked for staleness.", json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
                "body": {"type": "string"},
                "file": {"type": "string", "description": "file containing the symbol, relative to repo"},
                "symbol": {"type": "string", "description": "qualified symbol name, e.g. Foo::bar"},
                "repo": {"type": "string", "description": "repo root (defaults to the store root)"},
                "topic": {"type": "string"},
                "project": {"type": "string"},
                "by": {"type": "string"}
            },
            "required": ["kind","body","file","symbol"]
        })),
        tool("mem_read", "Read a single memory by id, returned as markdown.", json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        })),
        tool("mem_query", "Structured query over memories with an optional token budget.", filter_schema(false)),
        tool("mem_search", "Full-text search over memory bodies.", filter_schema(true)),
        tool("mem_supersede", "Replace an existing memory with a new one, recording the lineage.", json!({
            "type": "object",
            "properties": {
                "old_id": {"type": "string"},
                "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
                "body": {"type": "string"},
                "topic": {"type": "string"},
                "by": {"type": "string"}
            },
            "required": ["old_id","kind","body"]
        })),
        tool("mem_list_stale", "List code anchors whose referenced code has changed.", json!({
            "type": "object",
            "properties": {"repo": {"type": "string", "description": "repo root to check (defaults to the store root)"}}
        })),
        tool("mem_validate", "Validate every stored memory against its schema.", json!({"type": "object", "properties": {}})),
        tool("mem_status", "Summary counts of stored memories by kind.", json!({"type": "object", "properties": {}})),
    ])
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn filter_schema(with_text: bool) -> Value {
    let mut props = json!({
        "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
        "topic": {"type": "string"},
        "project": {"type": "string"},
        "tag": {"type": "string"},
        "min_confidence": {"type": "number"},
        "max_tokens": {"type": "integer"},
        "limit": {"type": "integer"},
        "include_expired": {"type": "boolean"}
    });
    let mut required = vec![];
    if with_text {
        props["text"] = json!({"type": "string"});
        required.push("text");
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// Run a tool by name. `Ok` text is the result; `Err` text is a tool error message.
pub fn call(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    let store = Store::open(root).map_err(|e| e.to_string())?;
    match name {
        "mem_write" => write(&store, args),
        "mem_anchor" => anchor(&store, root, args),
        "mem_read" => read(&store, args),
        "mem_query" => query(&store, args),
        "mem_search" => search(&store, args),
        "mem_supersede" => supersede(&store, args),
        "mem_list_stale" => list_stale(&store, root, args),
        "mem_validate" => validate(&store),
        "mem_status" => status(&store),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn write(store: &Store, args: &Value) -> Result<String, String> {
    let mut memory = memory_from(args)?;
    store.write(&mut memory).map_err(|e| e.to_string())
}

fn anchor(store: &Store, root: &Path, args: &Value) -> Result<String, String> {
    let file = str_arg(args, "file")?;
    let symbol = str_arg(args, "symbol")?;
    let repo = args
        .get("repo")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.to_path_buf());
    let core = seed_anchor(&repo, &file, &symbol)
        .ok_or_else(|| format!("symbol {symbol} not found in {file}"))?;
    let mut memory = memory_from(args)?;
    memory.frontmatter.refs.push(Ref {
        kind: RefKind::Symbol,
        value: format!("{file}::{symbol}"),
        anchor: Some(core.fingerprint.clone()),
    });
    memory.frontmatter.code_anchors.push(CodeAnchor {
        file_path: core.file_path,
        symbol: core.symbol,
        snippet: core.snippet,
        fingerprint: core.fingerprint,
        norm: core.norm,
    });
    store.write(&mut memory).map_err(|e| e.to_string())
}

fn read(store: &Store, args: &Value) -> Result<String, String> {
    let id = str_arg(args, "id")?;
    match store.read(&id).map_err(|e| e.to_string())? {
        Some(m) => Ok(marrow_memdocs::to_markdown(&m)),
        None => Err(format!("no memory with id {id}")),
    }
}

fn query(store: &Store, args: &Value) -> Result<String, String> {
    let hits = store.query(&query_from(args)).map_err(|e| e.to_string())?;
    Ok(summaries(&hits))
}

fn search(store: &Store, args: &Value) -> Result<String, String> {
    let text = str_arg(args, "text")?;
    let hits = store
        .search(&text, &query_from(args))
        .map_err(|e| e.to_string())?;
    Ok(summaries(&hits))
}

fn supersede(store: &Store, args: &Value) -> Result<String, String> {
    let old_id = str_arg(args, "old_id")?;
    let mut memory = memory_from(args)?;
    store
        .supersede(&old_id, &mut memory)
        .map_err(|e| e.to_string())
}

fn list_stale(store: &Store, root: &Path, args: &Value) -> Result<String, String> {
    let repo = args
        .get("repo")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.to_path_buf());
    let hits = store.list_stale(&repo).map_err(|e| e.to_string())?;
    let items: Vec<Value> = hits
        .iter()
        .map(|h| json!({"memory_id": h.memory_id, "symbol": h.symbol, "file_path": h.file_path, "relocated_to": h.relocated_to}))
        .collect();
    Ok(json!({"stale": items, "count": hits.len()}).to_string())
}

fn validate(store: &Store) -> Result<String, String> {
    let mut problems = Vec::new();
    for row in store.list().map_err(|e| e.to_string())? {
        if let Some(m) = store.read(&row.id).map_err(|e| e.to_string())? {
            if let Err(violations) = marrow_memdocs::validate(&m) {
                for v in violations {
                    problems.push(json!({"id": row.id, "field": v.field, "message": v.message}));
                }
            }
        }
    }
    Ok(json!({"problems": problems, "count": problems.len()}).to_string())
}

fn status(store: &Store) -> Result<String, String> {
    let rows = store.list().map_err(|e| e.to_string())?;
    let mut by_kind = serde_json::Map::new();
    for kind in ["fact", "decision", "entity", "session", "skill"] {
        let n = rows.iter().filter(|r| r.kind == kind).count();
        if n > 0 {
            by_kind.insert(kind.into(), json!(n));
        }
    }
    Ok(json!({"total": rows.len(), "by_kind": by_kind}).to_string())
}

fn summaries(hits: &[Memory]) -> String {
    let items: Vec<Value> = hits
        .iter()
        .map(|m| {
            json!({
                "id": m.frontmatter.id,
                "kind": kind_name(m.frontmatter.kind),
                "topic": m.frontmatter.topic,
                "body": m.body.trim(),
            })
        })
        .collect();
    json!({"results": items, "count": hits.len()}).to_string()
}

fn memory_from(args: &Value) -> Result<Memory, String> {
    let kind = parse_kind(&str_arg(args, "kind")?)?;
    let body = str_arg(args, "body")?;
    let tags = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(Memory {
        frontmatter: Frontmatter {
            id: String::new(),
            kind,
            status: Status::Active,
            topic: opt_arg(args, "topic"),
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: opt_arg(args, "project").unwrap_or_default(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: opt_arg(args, "by").unwrap_or_else(|| "mcp".into()),
                session_id: None,
                sources: vec![],
            },
            supersedes: vec![],
            tags,
            created_at: String::new(),
            updated_at: String::new(),
            hmac: None,
        },
        body,
    })
}

fn query_from(args: &Value) -> Query {
    Query {
        kind: opt_arg(args, "kind").and_then(|k| parse_kind(&k).ok()),
        status: Some(Status::Active),
        topic: opt_arg(args, "topic"),
        project_id: opt_arg(args, "project"),
        tag: opt_arg(args, "tag"),
        min_confidence: args.get("min_confidence").and_then(Value::as_f64),
        max_tokens: args
            .get("max_tokens")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
        limit: args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize),
        exclude_expired: !args
            .get("include_expired")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ..Query::default()
    }
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn opt_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(String::from)
}

fn parse_kind(s: &str) -> Result<MemoryKind, String> {
    Ok(match s {
        "fact" => MemoryKind::Fact,
        "decision" => MemoryKind::Decision,
        "entity" => MemoryKind::Entity,
        "session" => MemoryKind::Session,
        "skill" => MemoryKind::Skill,
        other => return Err(format!("unknown kind: {other}")),
    })
}

fn kind_name(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Fact => "fact",
        MemoryKind::Decision => "decision",
        MemoryKind::Entity => "entity",
        MemoryKind::Session => "session",
        MemoryKind::Skill => "skill",
    }
}
