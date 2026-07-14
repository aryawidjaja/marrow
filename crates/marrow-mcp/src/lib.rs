//! A Model Context Protocol server (spec 2025-06-18) over stdio for the Marrow store.
//!
//! Messages are newline-delimited JSON-RPC 2.0. [`handle`] processes one request and
//! returns the response to send (or `None` for notifications), so the protocol logic is
//! tested directly without spawning a process. [`serve`] is the stdio loop.

use std::io::{BufRead, Write};
use std::path::Path;

use serde_json::{json, Value};

pub mod prompts;
pub mod remote;
pub mod tools;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "marrow-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Handle one JSON-RPC message. Returns the response, or `None` for notifications.
pub fn handle(root: &Path, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "initialize" => {
            // The handshake says who is connecting. Remember it, or every memory an agent writes is
            // filed under "mcp" and you can never tell which agent knew what.
            if let Some(client) = req.pointer("/params/clientInfo") {
                tools::remember_client(client);
            }
            Some(success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {"tools": {}, "prompts": {}},
                    "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION}
                }),
            ))
        }
        "notifications/initialized" => None,
        "ping" => Some(success(id, json!({}))),
        "tools/list" => Some(success(id, json!({"tools": tools::definitions()}))),
        "tools/call" => Some(handle_call(root, id, req.get("params"))),
        "prompts/list" => Some(success(id, json!({"prompts": prompts::definitions()}))),
        "prompts/get" => Some(handle_prompt_get(id, req.get("params"))),
        _ if id.is_none() => None, // unknown notification
        _ => Some(error(id, -32601, &format!("method not found: {method}"))),
    }
}

fn handle_call(root: &Path, id: Option<Value>, params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match tools::dispatch(root, name, &args) {
        Ok(text) => tool_result(&text, false),
        Err(text) => tool_result(&text, true),
    };
    success(id, result)
}

fn handle_prompt_get(id: Option<Value>, params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match prompts::get(name) {
        Some(prompt) => success(id, prompt),
        None => error(id, -32602, &format!("unknown prompt: {name}")),
    }
}

/// Read newline-delimited JSON-RPC from `input`, writing responses to `output`.
pub fn serve(root: &Path, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            let resp = error(None, -32700, "parse error");
            writeln!(output, "{resp}")?;
            output.flush()?;
            continue;
        };
        if let Some(resp) = handle(root, &req) {
            writeln!(output, "{resp}")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn success(id: Option<Value>, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_store::Store;

    fn store_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Store::init(dir.path()).unwrap();
        dir
    }

    fn call(root: &Path, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
        handle(root, &req).unwrap()
    }

    fn result_text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn unqualified_search_is_bounded_by_default() {
        let dir = store_root();
        for i in 0..20 {
            call(
                dir.path(),
                "mem_write",
                json!({"kind":"fact","topic":format!("t{i}"),"body":format!("alpha fact number {i}")}),
            );
        }
        let resp = call(dir.path(), "mem_search", json!({"text":"alpha"}));
        let text = result_text(&resp);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        // No caller-supplied limit, yet the payload is capped, not the whole brain.
        assert!(v["results"].as_array().unwrap().len() <= 8, "got {}", text);
    }

    #[test]
    fn initialize_reports_protocol_and_server() {
        let dir = store_root();
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let resp = handle(dir.path(), &req).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(resp["result"]["serverInfo"]["name"], "marrow-mcp");
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialized_notification_has_no_response() {
        let dir = store_root();
        let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(handle(dir.path(), &req).is_none());
    }

    #[test]
    fn tools_list_advertises_the_lean_core_by_default() {
        let dir = store_root();
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let resp = handle(dir.path(), &req).unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // Core tools an agent actually calls are advertised...
        assert!(names.contains(&"mem_write"));
        assert!(names.contains(&"mem_recall"));
        assert!(names.contains(&"mem_bootstrap"));
        // ...while hook-driven coordination and rare inspection tools are hidden to save tokens.
        assert!(!names.contains(&"mem_claim"));
        assert!(!names.contains(&"mem_list_stale"));
    }

    #[test]
    fn full_catalog_covers_every_dispatchable_tool() {
        // Every advertised name in the full catalog must be dispatchable, and the full catalog
        // must be a superset of the lean core.
        let all = tools::all_definitions();
        let names: Vec<&str> = all.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"mem_list_stale"));
        assert!(names.contains(&"mem_claim"));
        assert!(names.len() > super::tools::definitions().as_array().unwrap().len());
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = store_root();
        let id = result_text(&call(
            dir.path(),
            "mem_write",
            json!({"kind":"decision","topic":"auth","body":"Use JWT for sessions."}),
        ));
        let id = id.trim();
        assert!(!id.is_empty());

        let resp = call(dir.path(), "mem_read", json!({ "id": id }));
        let text = result_text(&resp);
        assert!(text.contains("Use JWT"), "mem_read lost the body: {text}");
        assert_eq!(resp["result"]["isError"], false);

        // and it is findable without knowing the id
        let found = result_text(&call(dir.path(), "mem_search", json!({"text":"JWT"})));
        assert!(
            found.contains("Use JWT"),
            "mem_search could not find it: {found}"
        );
    }

    #[test]
    fn invalid_write_is_a_tool_error() {
        let dir = store_root();
        // A decision with no topic violates the schema.
        let resp = call(
            dir.path(),
            "mem_write",
            json!({"kind":"decision","body":"no topic"}),
        );
        assert_eq!(resp["result"]["isError"], true);
        assert!(result_text(&resp).contains("topic"));
    }

    #[test]
    fn search_finds_body_text() {
        let dir = store_root();
        call(
            dir.path(),
            "mem_write",
            json!({"kind":"fact","topic":"net","body":"rate limits use a token bucket"}),
        );
        let resp = call(dir.path(), "mem_search", json!({"text":"token bucket"}));
        assert!(result_text(&resp).contains("token bucket"));
    }

    #[test]
    fn history_and_audit_tools_work() {
        let dir = store_root();
        call(
            dir.path(),
            "mem_write",
            json!({"kind":"fact","topic":"x","body":"a recorded fact"}),
        );
        let hist = call(dir.path(), "mem_history", json!({}));
        assert!(result_text(&hist).contains("\"kind\":\"write\""));
        let audit = call(dir.path(), "mem_audit", json!({}));
        assert!(result_text(&audit).contains("\"ok\":true"));
    }

    #[test]
    fn coordination_tools_round_trip() {
        let dir = store_root();
        // Agent A claims the auth work.
        let c = call(
            dir.path(),
            "mem_claim",
            json!({"session":"a","intent":"refactor auth","files":["src/auth.rs"],"project":"demo"}),
        );
        let claim_id = result_text(&c);
        assert!(claim_id.contains("\"id\""));

        // Agent B checks before touching the same file and sees the collision.
        let overlap = call(
            dir.path(),
            "mem_claims",
            json!({"files":["src/auth.rs"],"project":"demo"}),
        );
        assert!(result_text(&overlap).contains("refactor auth"));
        assert!(result_text(&overlap).contains("\"count\":1"));

        // Activity stream shows the claim.
        let act = call(dir.path(), "mem_activity", json!({}));
        assert!(result_text(&act).contains("\"kind\":\"claim\""));

        // Bootstrap warm-starts a fresh session with the active claim.
        let brief = call(
            dir.path(),
            "mem_bootstrap",
            json!({"goal":"work on auth","project":"demo"}),
        );
        assert!(result_text(&brief).contains("refactor auth"));
    }

    #[test]
    fn writing_with_an_anchor_tracks_code_staleness() {
        let dir = store_root();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/auth.rs"),
            "pub fn issue_token(u: &str) -> String { format!(\"jwt:{u}\") }\n",
        )
        .unwrap();

        // Anchoring lives on mem_write: a memory about code gets tied to that code as it is written.
        // It was a separate tool once, and that tool was never called, so staleness never fired.
        let resp = call(
            dir.path(),
            "mem_write",
            json!({"kind":"decision","topic":"auth","area":"auth","body":"Issues a JWT.",
                   "anchor":{"file":"src/auth.rs","symbol":"issue_token"}}),
        );
        assert_eq!(resp["result"]["isError"], false);

        let fresh = call(dir.path(), "mem_list_stale", json!({}));
        assert!(result_text(&fresh).contains("\"count\":0"));

        std::fs::write(
            dir.path().join("src/auth.rs"),
            "pub fn issue_token(u: &str) -> String { format!(\"opaque:{u}:v2\") }\n",
        )
        .unwrap();
        let stale = call(dir.path(), "mem_list_stale", json!({}));
        assert!(result_text(&stale).contains("\"count\":1"));
        assert!(result_text(&stale).contains("issue_token"));

        // ...and the next session is TOLD, without having to call a tool it might never call.
        let brief = call(dir.path(), "mem_bootstrap", json!({"goal": "work on auth"}));
        let text = result_text(&brief);
        assert!(
            text.contains("issue_token"),
            "warm start must surface the stale memory: {text}"
        );
        assert!(text.contains("stale_note"));
    }

    #[test]
    fn initialize_advertises_prompts_capability() {
        let dir = store_root();
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let resp = handle(dir.path(), &req).unwrap();
        assert!(resp["result"]["capabilities"]["prompts"].is_object());
    }

    #[test]
    fn prompts_list_and_get_the_save_prompt() {
        let dir = store_root();
        let list = handle(
            dir.path(),
            &json!({"jsonrpc":"2.0","id":1,"method":"prompts/list"}),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"save"));

        let got = handle(
            dir.path(),
            &json!({"jsonrpc":"2.0","id":2,"method":"prompts/get","params":{"name":"save"}}),
        )
        .unwrap();
        assert!(got["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("mem_write"));

        let missing = handle(
            dir.path(),
            &json!({"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"nope"}}),
        )
        .unwrap();
        assert_eq!(missing["error"]["code"], -32602);
    }

    #[test]
    fn ingest_tool_lists_docs() {
        let dir = store_root();
        std::fs::write(dir.path().join("README.md"), "# hi").unwrap();
        let resp = call(dir.path(), "mem_ingest", json!({}));
        let text = result_text(&resp);
        assert!(text.contains("README.md"));
        assert!(text.contains("mem_write"));
        assert!(text.contains("\"count\":1"));
    }

    #[test]
    fn unknown_method_returns_protocol_error() {
        let dir = store_root();
        let req = json!({"jsonrpc":"2.0","id":9,"method":"does/not/exist"});
        let resp = handle(dir.path(), &req).unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[test]
    fn serve_processes_a_session() {
        let dir = store_root();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );
        let mut out = Vec::new();
        serve(dir.path(), std::io::Cursor::new(input), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // initialize -> 1 response; initialized -> none; tools/list -> 1 response.
        assert_eq!(lines.len(), 2, "got: {text}");
        assert!(lines[0].contains("protocolVersion"));
        assert!(lines[1].contains("mem_write"));
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;
    use marrow_store::Store;
    use serde_json::json;

    fn store_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Store::init(dir.path()).unwrap();
        dir
    }
    fn call(root: &Path, name: &str, args: Value) -> Value {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
        handle(root, &req).unwrap()
    }
    fn text(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn recall_then_provenance_shows_retrieval() {
        let dir = store_root();
        let id = text(&call(
            dir.path(),
            "mem_write",
            json!({"kind":"fact","topic":"x","body":"the cache ttl is 60s"}),
        ));
        call(
            dir.path(),
            "mem_recall",
            json!({"text":"cache ttl","by":"agent"}),
        );
        let prov = call(dir.path(), "mem_provenance", json!({"id": id.trim()}));
        assert!(text(&prov).contains("\"kind\":\"retrieve\""));
        assert!(text(&prov).contains("written_by"));
    }
}
