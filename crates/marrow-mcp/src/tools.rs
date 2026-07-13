//! Tool definitions and execution against a Marrow store.

use std::path::{Path, PathBuf};

use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::{knowledge_docs, ClaimScope, Hub, Query, Store};
use serde_json::{json, Value};

/// The tool catalog advertised via `tools/list`.
///
/// By default only the core tools an agent reaches for mid-work are advertised — the coordination
/// tools (claim/release/claims/progress/activity) are driven by the hooks via the CLI, not the
/// agent, and the inspection tools (audit/validate/history/…) are rarely needed. Every session
/// pays for each advertised schema, so the lean catalog is the default. Set `MARROW_TOOLS=full`
/// to advertise all of them. Hidden tools remain callable — [`call`] dispatches by name regardless.
pub fn definitions() -> Value {
    let full = std::env::var("MARROW_TOOLS").is_ok_and(|v| v == "full");
    let all = all_definitions();
    if full {
        return Value::Array(all);
    }
    // The cross-project tools cost nothing until a hive exists, so advertise them only when the
    // machine actually has registered projects. `active` checks without creating anything on disk.
    let hub_active = marrow_store::Hub::active();
    Value::Array(
        all.into_iter()
            .filter(|t| {
                let name = t["name"].as_str().unwrap_or_default();
                CORE_TOOLS.contains(&name) || (hub_active && HUB_TOOLS.contains(&name))
            })
            .collect(),
    )
}

/// Tools advertised by default: what an agent actually calls while working.
const CORE_TOOLS: &[&str] = &[
    "mem_bootstrap",
    "mem_recall",
    "mem_search",
    "mem_areas",
    "mem_write",
    "mem_read",
    "mem_supersede",
    "mem_ingest",
];

/// Hive tools — cross-project recall/awareness and the agent channel — advertised only when a hive
/// is configured (see [`definitions`]).
const HUB_TOOLS: &[&str] = &[
    "mem_hub_recall",
    "mem_hub_activity",
    "mem_ask",
    "mem_inbox",
    "mem_reply",
];

pub(crate) fn all_definitions() -> Vec<Value> {
    let Value::Array(defs) = json!([
        tool("mem_write", "Write a new memory (fact, decision, entity, session, or skill). Rejects invalid writes with the reasons. FILE IT: pass `area` so the memory lands in the right part of the project's brain — call mem_areas first and REUSE an existing area rather than inventing a near-duplicate.", json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
                "body": {"type": "string"},
                "topic": {"type": "string", "description": "Short label for what this is about (max 48 chars), e.g. `jwt-expiry`. NOT a sentence — the detail goes in the body. Memories on the same topic supersede each other."},
                "area": {"type": "string", "description": "The feature area this belongs to, e.g. `auth`, `billing`, `infra`. Call mem_areas and reuse one of the project's existing areas; only invent a new one if nothing fits. Leave it out if genuinely nothing fits — an unfiled memory is still fully searchable, and a wrong area is worse than none."},
                "project": {"type": "string"},
                "by": {"type": "string", "description": "author for provenance"},
                "tags": {"type": "array", "items": {"type": "string"}}
            },
            "required": ["kind","body"]
        })),
        tool("mem_areas", "The table of contents for this project's brain: its feature areas and how many memories each holds. Call this before mem_write so you file the memory into an area that already exists.", json!({
            "type": "object", "properties": {}
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
        tool("mem_search", "Hybrid keyword+semantic search over memories.", filter_schema(true)),
        tool("mem_recall", "Recall memories AND the ones connected to them (explicit links, shared topic/tag, related meaning) in one call — the related cluster, not just the matches. Records the retrieval so answers stay traceable.", filter_schema(true)),
        tool("mem_provenance", "Trace a memory's origin, lineage, and how it has been used.", json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        })),
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
        tool("mem_history", "Read the episodic / audit history (most recent last).", json!({
            "type": "object",
            "properties": {"limit": {"type": "integer"}}
        })),
        tool("mem_audit", "Verify the audit chain is intact and tamper-free.", json!({"type": "object", "properties": {}})),
        tool("mem_consolidate", "Detect (or with apply=true, perform) consolidation: stale anchors, expired memories, and duplicate clusters.", json!({
            "type": "object",
            "properties": {
                "repo": {"type": "string", "description": "repo root for staleness (defaults to store root)"},
                "apply": {"type": "boolean", "description": "merge duplicates and retire expired instead of only reporting"}
            }
        })),
        tool("mem_log", "Append an agent-authored event (observation, correction, note).", json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string"},
                "summary": {"type": "string"},
                "by": {"type": "string"}
            },
            "required": ["summary"]
        })),
        tool("mem_claim", "Register an advisory work-claim so other agent sessions know what you're working on and don't collide. Returns the claim id.", json!({
            "type": "object",
            "properties": {
                "session": {"type": "string", "description": "your session id"},
                "intent": {"type": "string", "description": "what you're about to do"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "files or dir/* globs you'll touch"},
                "symbols": {"type": "array", "items": {"type": "string"}},
                "topic": {"type": "string"},
                "feature": {"type": "string"},
                "project": {"type": "string"},
                "ttl_secs": {"type": "integer", "description": "lease length in seconds (default 900; renews on progress)"},
                "by": {"type": "string"}
            },
            "required": ["session","intent"]
        })),
        tool("mem_release", "Release a work-claim you no longer need (otherwise it expires at its TTL).", json!({
            "type": "object",
            "properties": {"claim_id": {"type": "string"}, "by": {"type": "string"}},
            "required": ["claim_id"]
        })),
        tool("mem_claims", "List active work-claims. With a scope (files/symbols/topic/feature), returns only claims that would collide — check this BEFORE starting work.", json!({
            "type": "object",
            "properties": {
                "files": {"type": "array", "items": {"type": "string"}},
                "symbols": {"type": "array", "items": {"type": "string"}},
                "topic": {"type": "string"},
                "feature": {"type": "string"},
                "project": {"type": "string"}
            }
        })),
        tool("mem_progress", "Record a unit of progress (what you just did, which files) so other sessions see it in the activity stream.", json!({
            "type": "object",
            "properties": {
                "summary": {"type": "string"},
                "session": {"type": "string"},
                "files": {"type": "array", "items": {"type": "string"}},
                "by": {"type": "string"}
            },
            "required": ["summary"]
        })),
        tool("mem_activity", "The most recent activity-stream events across all sessions (newest first).", json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "description": "max events (default 20)"}}
        })),
        tool("mem_bootstrap", "Warm-start a session: announce it and get back what other sessions are doing plus the memories and decisions most relevant to your goal. Call this FIRST instead of re-scanning.", json!({
            "type": "object",
            "properties": {
                "goal": {"type": "string"},
                "project": {"type": "string"},
                "max_tokens": {"type": "integer", "description": "budget for relevant memories (default 1500)"},
                "by": {"type": "string"}
            },
            "required": ["goal"]
        })),
        tool("mem_ingest", "Onboard an existing repo: list its knowledge docs (READMEs, docs/) so you can distill them into memory. Read each and save the durable decisions/facts with mem_write — distill, don't dump.", json!({
            "type": "object",
            "properties": {}
        })),
        tool("mem_hub_recall", "Recall across EVERY project on this machine, not just this one — ask what the whole hive knows. Results are tagged with the project they came from.", json!({
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "limit": {"type": "integer", "description": "max results (default 8)"}
            },
            "required": ["text"]
        })),
        tool("mem_hub_activity", "What agent sessions in OTHER projects are doing right now (newest first), tagged by project — the cross-project pulse of the hive.", json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "description": "max events (default 20)"}}
        })),
        tool("mem_ask", "Ask another agent (in this or another project) a question, or share a note — a message in the shared channel. `to` is who: a session id, a name, a project, or \"all\". Non-destructive; the human can read it.", json!({
            "type": "object",
            "properties": {
                "to": {"type": "string", "description": "recipient: session id, name, project, or \"all\""},
                "body": {"type": "string"},
                "by": {"type": "string", "description": "your name/session"}
            },
            "required": ["to","body"]
        })),
        tool("mem_inbox", "Messages waiting for you from other agents (newest first) — check this to see if anyone asked you something. Pass your session/name so it knows what's addressed to you.", json!({
            "type": "object",
            "properties": {
                "session": {"type": "string"},
                "by": {"type": "string"},
                "project": {"type": "string"}
            }
        })),
        tool("mem_reply", "Reply to a message thread (from mem_inbox). The reply goes back to whoever asked.", json!({
            "type": "object",
            "properties": {
                "thread": {"type": "string"},
                "body": {"type": "string"},
                "by": {"type": "string"}
            },
            "required": ["thread","body"]
        })),
    ]) else {
        unreachable!("tool catalog is a JSON array literal")
    };
    defs
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn filter_schema(with_text: bool) -> Value {
    let mut props = json!({
        "kind": {"type": "string", "enum": ["fact","decision","entity","session","skill"]},
        "topic": {"type": "string"},
        "area": {"type": "string", "description": "Feature area to favour (auth, billing, infra). This BOOSTS that area's memories to the top; it does not hide the rest."},
        "project": {"type": "string"},
        "tag": {"type": "string"},
        "min_confidence": {"type": "number"},
        "max_tokens": {"type": "integer"},
        "limit": {"type": "integer"},
        "include_expired": {"type": "boolean"},
        "weight": {"type": "number", "description": "hybrid search weight: 0 keyword, 1 semantic"}
    });
    let mut required = vec![];
    if with_text {
        props["text"] = json!({"type": "string"});
        required.push("text");
    }
    json!({"type": "object", "properties": props, "required": required})
}

/// Entry point for the stdio MCP server. Routing is per project: if *this* project has been shared
/// (`.marrow/remote.toml`), its calls go to that gateway space; otherwise, if the whole machine is
/// wired remote (`MARROW_REMOTE`), calls go there; otherwise they run against the local store. So a
/// shared project reaches its shared brain while every other project stays local and private. The
/// backbone itself calls [`call`] directly, so it never recurses.
pub fn dispatch(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    if let Some(remote) = marrow_store::SharedRemote::load(root) {
        return crate::remote::forward_to(
            &remote.url,
            remote.token.as_deref(),
            &remote.space,
            name,
            args,
        );
    }
    if crate::remote::endpoint().is_some() {
        return crate::remote::forward(name, args);
    }
    call(root, name, args)
}

/// Run a tool by name against the local store. `Ok` text is the result; `Err` text is a tool error.
pub fn call(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    // Cross-project tools reach the whole hive, not this one store — handle them first.
    match name {
        "mem_hub_recall" => return hub_recall(args),
        "mem_hub_activity" => return hub_activity(args),
        "mem_ask" => return ask(root, args),
        "mem_inbox" => return inbox(root, args),
        "mem_reply" => return reply(root, args),
        _ => {}
    }
    let store = Store::open(root).map_err(|e| e.to_string())?;
    match name {
        "mem_write" => write(&store, args),
        "mem_anchor" => anchor(&store, root, args),
        "mem_read" => read(&store, args),
        "mem_query" => query(&store, args),
        "mem_search" => search(&store, args),
        "mem_areas" => areas(&store),
        "mem_recall" => recall(&store, args),
        "mem_provenance" => provenance(&store, args),
        "mem_supersede" => supersede(&store, args),
        "mem_list_stale" => list_stale(&store, root, args),
        "mem_validate" => validate(&store),
        "mem_status" => status(&store),
        "mem_history" => history(&store, args),
        "mem_audit" => audit(&store),
        "mem_log" => log_event(&store, args),
        "mem_consolidate" => consolidate(&store, root, args),
        "mem_claim" => claim(&store, args),
        "mem_release" => release(&store, args),
        "mem_claims" => claims(&store, args),
        "mem_progress" => progress(&store, args),
        "mem_activity" => activity(&store, args),
        "mem_bootstrap" => bootstrap(&store, args),
        "mem_ingest" => ingest(root),
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
    let mut memory = memory_from(args)?;
    store
        .write_anchored(&repo, &file, &symbol, &mut memory)
        .map_err(|e| e.to_string())
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

/// The project's table of contents: its feature areas and how many memories each holds. The agent
/// reads this before writing so it files into an area that already exists.
fn areas(store: &Store) -> Result<String, String> {
    let all = store.areas().map_err(|e| e.to_string())?;
    let filed: Vec<Value> = all
        .iter()
        .filter(|(a, _)| !a.is_empty())
        .map(|(a, n)| json!({"area": a, "memories": n}))
        .collect();
    let unfiled = all
        .iter()
        .find(|(a, _)| a.is_empty())
        .map(|(_, n)| *n)
        .unwrap_or(0);
    let guidance = if filed.len() >= 12 {
        "This project already has a lot of areas. Reuse one of the above; do NOT add another unless nothing fits."
    } else {
        "Reuse one of these areas when you mem_write. Only invent a new area if none genuinely fits — and keep the project under ~12."
    };
    Ok(json!({"areas": filed, "unfiled": unfiled, "guidance": guidance}).to_string())
}

fn recall(store: &Store, args: &Value) -> Result<String, String> {
    let text = str_arg(args, "text")?;
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    // Associative recall: the matches plus the memories connected to them (one fetch, the related
    // cluster). `connect` caps the extras (0 turns it off); neighbours come back terse.
    let max_neighbors = args
        .get("connect")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(8);
    let r = store
        .recall_connected(&text, &query_from(args), &by, max_neighbors)
        .map_err(|e| e.to_string())?;
    let mut seeds = r.seeds.clone();
    // `area` is a BOOST, never a filter: memories in the named area float to the top, but everything
    // else still comes back. A memory filed in the wrong area must never become unfindable.
    if let Some(area) = opt_arg(args, "area").filter(|a| !a.is_empty()) {
        seeds.sort_by_key(|m| m.frontmatter.area.as_deref() != Some(area.as_str()));
    }
    let results: Vec<Value> = seeds
        .iter()
        .map(|m| {
            json!({
                "id": m.frontmatter.id,
                "kind": kind_name(m.frontmatter.kind),
                "topic": m.frontmatter.topic,
                "area": m.frontmatter.area,
                "body": m.body.trim(),
            })
        })
        .collect();
    let connected: Vec<Value> = r
        .neighbors
        .iter()
        .map(|n| {
            json!({
                "id": n.memory.frontmatter.id,
                "kind": kind_name(n.memory.frontmatter.kind),
                "topic": n.memory.frontmatter.topic,
                "via": n.via,
                "snippet": n.memory.body.trim().lines().next().unwrap_or("").chars().take(140).collect::<String>(),
            })
        })
        .collect();
    Ok(json!({"results": results, "count": results.len(), "connected": connected}).to_string())
}

fn provenance(store: &Store, args: &Value) -> Result<String, String> {
    let id = str_arg(args, "id")?;
    match store.provenance(&id).map_err(|e| e.to_string())? {
        Some(t) => {
            let mem_ref = |r: &marrow_store::MemoryRef| json!({"id": r.id, "kind": r.kind, "topic": r.topic, "status": r.status});
            Ok(json!({
                "id": t.id,
                "written_by": t.written_by,
                "sources": t.sources,
                "supersedes": t.supersedes.iter().map(mem_ref).collect::<Vec<_>>(),
                "superseded_by": t.superseded_by.iter().map(mem_ref).collect::<Vec<_>>(),
                "history": t.events.iter().map(|e| json!({
                    "seq": e.seq, "ts": e.ts, "kind": e.kind, "summary": e.summary
                })).collect::<Vec<_>>(),
            })
            .to_string())
        }
        None => Err(format!("no memory with id {id}")),
    }
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

fn history(store: &Store, args: &Value) -> Result<String, String> {
    let events = store.history().map_err(|e| e.to_string())?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let start = limit.map(|n| events.len().saturating_sub(n)).unwrap_or(0);
    let items: Vec<Value> = events[start..]
        .iter()
        .map(|e| {
            json!({
                "seq": e.seq, "ts": e.ts, "kind": e.kind, "actor": e.actor,
                "memory_id": e.memory_id, "summary": e.summary,
            })
        })
        .collect();
    Ok(json!({"events": items, "total": events.len()}).to_string())
}

fn consolidate(store: &Store, root: &Path, args: &Value) -> Result<String, String> {
    let repo = args
        .get("repo")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| root.to_path_buf());
    if args.get("apply").and_then(Value::as_bool).unwrap_or(false) {
        let o = store.consolidate_apply(&repo).map_err(|e| e.to_string())?;
        Ok(json!({
            "applied": true,
            "deprecated": o.deprecated,
            "merged": o.merged,
            "conflicts_resolved": o.conflicts_resolved,
        })
        .to_string())
    } else {
        let r = store.consolidate(&repo).map_err(|e| e.to_string())?;
        let related: usize = r.clusters.iter().map(|c| c.others.len()).sum();
        Ok(json!({
            "stale": r.stale.len(),
            "expired": r.expired.len(),
            "related_memories": related,
            "clusters": r.clusters.len(),
        })
        .to_string())
    }
}

fn audit(store: &Store) -> Result<String, String> {
    match store.verify_log() {
        Ok(()) => Ok(json!({"ok": true}).to_string()),
        Err(seq) => Ok(json!({"ok": false, "broken_at_seq": seq}).to_string()),
    }
}

fn log_event(store: &Store, args: &Value) -> Result<String, String> {
    let summary = str_arg(args, "summary")?;
    let kind = opt_arg(args, "kind").unwrap_or_else(|| "observe".into());
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    store
        .log_event(&kind, &by, &summary)
        .map_err(|e| e.to_string())?;
    Ok("logged".to_string())
}

fn claim(store: &Store, args: &Value) -> Result<String, String> {
    let session = str_arg(args, "session")?;
    let intent = str_arg(args, "intent")?;
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    let ttl = args.get("ttl_secs").and_then(Value::as_i64).unwrap_or(900);
    let c = store
        .claim(&session, &by, scope_from(args), &intent, ttl)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&c)
        .map(|v| v.to_string())
        .map_err(|e| e.to_string())
}

fn release(store: &Store, args: &Value) -> Result<String, String> {
    let claim_id = str_arg(args, "claim_id")?;
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    store.release(&claim_id, &by).map_err(|e| e.to_string())?;
    Ok(json!({"released": claim_id}).to_string())
}

fn claims(store: &Store, args: &Value) -> Result<String, String> {
    let scope = scope_from(args);
    let found = if scope_is_empty(&scope) {
        store.active_claims()
    } else {
        store.claims_overlapping(&scope)
    }
    .map_err(|e| e.to_string())?;
    let value = serde_json::to_value(&found).map_err(|e| e.to_string())?;
    Ok(json!({"claims": value, "count": found.len()}).to_string())
}

fn progress(store: &Store, args: &Value) -> Result<String, String> {
    let summary = str_arg(args, "summary")?;
    let session = opt_arg(args, "session").unwrap_or_else(|| "mcp".into());
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    let files = arr_arg(args, "files");
    store
        .progress(&session, &by, &summary, &files)
        .map_err(|e| e.to_string())?;
    Ok(json!({"recorded": true}).to_string())
}

fn activity(store: &Store, args: &Value) -> Result<String, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(20);
    let events = store.activity(limit).map_err(|e| e.to_string())?;
    let items: Vec<Value> = events
        .iter()
        .map(|e| {
            json!({
                "seq": e.seq, "ts": e.ts, "kind": e.kind, "actor": e.actor,
                "summary": e.summary, "data": e.data,
            })
        })
        .collect();
    Ok(json!({"events": items, "count": items.len()}).to_string())
}

fn bootstrap(store: &Store, args: &Value) -> Result<String, String> {
    let goal = str_arg(args, "goal")?;
    let project = opt_arg(args, "project").unwrap_or_else(|| "default".into());
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    let max_tokens = args
        .get("max_tokens")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(1500);
    let brief = store
        .bootstrap(&goal, &project, &by, max_tokens)
        .map_err(|e| e.to_string())?;
    let claims = serde_json::to_value(&brief.active_claims).map_err(|e| e.to_string())?;
    // The map of the project's brain, so the agent knows what exists before it recalls or writes.
    let areas: Vec<Value> = store
        .areas()
        .unwrap_or_default()
        .into_iter()
        .filter(|(a, _)| !a.is_empty())
        .map(|(a, n)| json!({"area": a, "memories": n}))
        .collect();
    Ok(json!({
        "goal": brief.goal,
        "areas": areas,
        "active_claims": claims,
        "relevant": brief.relevant.iter().map(mem_brief).collect::<Vec<_>>(),
        "recent_decisions": brief.recent_decisions.iter().map(mem_brief_snippet).collect::<Vec<_>>(),
        "suggest_ingest": brief.suggest_ingest,
        "suggest_consolidate": brief.suggest_consolidate,
    })
    .to_string())
}

/// List the project's knowledge docs plus a directive to distill them into memory. Marrow runs no
/// LLM here — the agent reads the docs and writes the memories itself.
fn ingest(root: &Path) -> Result<String, String> {
    let docs = knowledge_docs(root);
    let files: Vec<Value> = docs
        .iter()
        .map(|(path, bytes)| json!({"path": path, "bytes": bytes}))
        .collect();
    let instruction = if docs.is_empty() {
        "No knowledge docs (Markdown) found under this project.".to_string()
    } else {
        "Read each file and save the durable decisions, facts, and architecture with mem_write — \
         distill, don't paste whole files. Call mem_recall first and skip anything already saved."
            .to_string()
    };
    Ok(json!({"instruction": instruction, "docs": files, "count": docs.len()}).to_string())
}

fn hub_recall(args: &Value) -> Result<String, String> {
    let text = str_arg(args, "text")?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(8);
    let hub = Hub::open().map_err(|e| e.to_string())?;
    let items: Vec<Value> = hub
        .recall(&text, limit, limit)
        .iter()
        .map(|h| {
            json!({
                "project": h.project,
                "id": h.memory.frontmatter.id,
                "kind": kind_name(h.memory.frontmatter.kind),
                "topic": h.memory.frontmatter.topic,
                "body": h.memory.body.trim(),
            })
        })
        .collect();
    Ok(json!({"results": items, "count": items.len()}).to_string())
}

/// The message channel lives in the shared hub `core` store when a hive exists (so agents in
/// different projects share it), otherwise in this project's store.
fn channel_store(root: &Path) -> Result<Store, String> {
    if marrow_store::Hub::active() {
        Hub::open()
            .and_then(|h| h.core())
            .map_err(|e| e.to_string())
    } else {
        Store::open(root).map_err(|e| e.to_string())
    }
}

fn ask(root: &Path, args: &Value) -> Result<String, String> {
    let to = str_arg(args, "to")?;
    let body = str_arg(args, "body")?;
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    let thread = channel_store(root)?
        .post_message(&by, &to, None, "ask", &body)
        .map_err(|e| e.to_string())?;
    Ok(json!({"posted": true, "thread": thread}).to_string())
}

fn reply(root: &Path, args: &Value) -> Result<String, String> {
    let thread = str_arg(args, "thread")?;
    let body = str_arg(args, "body")?;
    let by = opt_arg(args, "by").unwrap_or_else(|| "mcp".into());
    let store = channel_store(root)?;
    // A reply goes back to whoever started the thread (unless a recipient is named).
    let to = opt_arg(args, "to").or_else(|| {
        store
            .thread(&thread)
            .ok()
            .and_then(|ms| ms.into_iter().map(|m| m.from).find(|f| f != &by))
    });
    let to = to.unwrap_or_else(|| "all".into());
    let t = store
        .post_message(&by, &to, Some(&thread), "reply", &body)
        .map_err(|e| e.to_string())?;
    Ok(json!({"posted": true, "thread": t}).to_string())
}

fn inbox(root: &Path, args: &Value) -> Result<String, String> {
    let me: Vec<String> = ["session", "by", "project"]
        .iter()
        .filter_map(|k| opt_arg(args, k))
        .collect();
    let me = if me.is_empty() {
        vec!["mcp".into()]
    } else {
        me
    };
    let msgs = channel_store(root)?
        .inbox(&me, 10)
        .map_err(|e| e.to_string())?;
    let items: Vec<Value> = msgs
        .iter()
        .map(|m| json!({"thread": m.thread, "from": m.from, "role": m.role, "body": m.body}))
        .collect();
    Ok(json!({"messages": items, "count": items.len()}).to_string())
}

fn hub_activity(args: &Value) -> Result<String, String> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(20);
    let hub = Hub::open().map_err(|e| e.to_string())?;
    let items: Vec<Value> = hub
        .activity(limit)
        .iter()
        .map(|e| {
            json!({
                "project": e.project,
                "ts": e.event.ts,
                "kind": e.event.kind,
                "summary": e.event.summary,
            })
        })
        .collect();
    Ok(json!({"events": items, "count": items.len()}).to_string())
}

fn mem_brief(m: &Memory) -> Value {
    json!({
        "id": m.frontmatter.id,
        "kind": kind_name(m.frontmatter.kind),
        "topic": m.frontmatter.topic,
        "body": m.body.trim(),
    })
}

/// Like [`mem_brief`] but the body is a length-capped snippet — the warm-start briefing lists
/// recent decisions for orientation; the agent reads the full one by id only if it matters.
fn mem_brief_snippet(m: &Memory) -> Value {
    let body = m.body.trim();
    let capped: String = body.chars().take(220).collect();
    let capped = if body.chars().count() > 220 {
        format!("{}…", capped.trim_end())
    } else {
        capped
    };
    json!({
        "id": m.frontmatter.id,
        "kind": kind_name(m.frontmatter.kind),
        "topic": m.frontmatter.topic,
        "body": capped,
    })
}

fn scope_from(args: &Value) -> ClaimScope {
    ClaimScope {
        files: arr_arg(args, "files"),
        symbols: arr_arg(args, "symbols"),
        topic: opt_arg(args, "topic"),
        feature: opt_arg(args, "feature"),
        project_id: opt_arg(args, "project").unwrap_or_default(),
    }
}

fn scope_is_empty(s: &ClaimScope) -> bool {
    s.files.is_empty() && s.symbols.is_empty() && s.topic.is_none() && s.feature.is_none()
}

fn arr_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
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
            area: opt_arg(args, "area"),
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

/// Default caps on a retrieval so an unqualified `mem_search`/`mem_recall` can't dump the whole
/// brain into context. Generous enough to answer most questions; the caller can raise either.
const DEFAULT_MAX_TOKENS: usize = 1200;
const DEFAULT_LIMIT: usize = 8;

fn query_from(args: &Value) -> Query {
    Query {
        kind: opt_arg(args, "kind").and_then(|k| parse_kind(&k).ok()),
        status: Some(Status::Active),
        topic: opt_arg(args, "topic"),
        project_id: opt_arg(args, "project"),
        tag: opt_arg(args, "tag"),
        min_confidence: args.get("min_confidence").and_then(Value::as_f64),
        max_tokens: Some(
            args.get("max_tokens")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_TOKENS),
        ),
        limit: Some(
            args.get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_LIMIT),
        ),
        exclude_expired: !args
            .get("include_expired")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hybrid_weight: args.get("weight").and_then(Value::as_f64),
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
