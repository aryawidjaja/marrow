//! The Marrow served backbone.
//!
//! A single HTTP endpoint, `POST /v1/rpc`, carries the same tool calls the local MCP server runs —
//! so a device points its Marrow at this backbone and every device sharing a project shares one
//! brain. Requests are routed to a per-project store under a server-side data dir; a bearer token
//! guards writes and reads. [`handle`] is a plain function so the whole API is testable without a
//! socket; [`serve`] runs the tiny_http loop.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Server configuration.
pub struct Config {
    /// Directory holding one store per project (`<data_dir>/<project>/.marrow`).
    pub data_dir: PathBuf,
    /// Shared bearer token. When set, every request except `/health` must present it.
    pub token: Option<String>,
}

/// A ready-to-send response.
pub struct Reply {
    pub status: u16,
    pub body: String,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: body.to_string(),
    }
}

/// The default project when a request names none.
const DEFAULT_PROJECT: &str = "default";

/// Reject anything that isn't a plain project slug, so `project` can never escape the data dir.
fn safe_project(name: &str) -> Option<String> {
    let name = if name.is_empty() {
        DEFAULT_PROJECT
    } else {
        name
    };
    let ok = name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.');
    ok.then(|| name.to_string())
}

fn store_root(cfg: &Config, project: &str) -> Result<PathBuf, String> {
    let project = safe_project(project).ok_or("invalid project name")?;
    let root = cfg.data_dir.join(project);
    if !root.join(".marrow").is_dir() {
        marrow_store::Store::init(&root).map_err(|e| e.to_string())?;
    }
    Ok(root)
}

fn authorized(cfg: &Config, auth: Option<&str>) -> bool {
    match &cfg.token {
        None => true,
        Some(t) => auth
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|got| got == t)
            .unwrap_or(false),
    }
}

/// Handle one request. `auth` is the Authorization header value, `body` the raw request body.
pub fn handle(cfg: &Config, method: &str, path: &str, auth: Option<&str>, body: &str) -> Reply {
    if method == "GET" && path == "/health" {
        return reply(
            200,
            json!({"ok": true, "service": "marrow-server", "version": env!("CARGO_PKG_VERSION")}),
        );
    }
    if !authorized(cfg, auth) {
        return reply(401, json!({"ok": false, "error": "unauthorized"}));
    }
    match (method, path) {
        ("POST", "/v1/rpc") => rpc(cfg, body),
        _ => reply(404, json!({"ok": false, "error": "not found"})),
    }
}

fn rpc(cfg: &Config, body: &str) -> Reply {
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return reply(400, json!({"ok": false, "error": format!("bad json: {e}")})),
    };
    let Some(tool) = req.get("tool").and_then(Value::as_str) else {
        return reply(400, json!({"ok": false, "error": "missing 'tool'"}));
    };
    let args = req.get("args").cloned().unwrap_or_else(|| json!({}));
    let project = req.get("project").and_then(Value::as_str).unwrap_or("");
    let root = match store_root(cfg, project) {
        Ok(r) => r,
        Err(e) => return reply(400, json!({"ok": false, "error": e})),
    };
    match marrow_mcp::tools::call(&root, tool, &args) {
        Ok(text) => reply(200, json!({"ok": true, "result": passthrough(&text)})),
        Err(e) => reply(200, json!({"ok": false, "error": e})),
    }
}

/// Tool results are already JSON strings; re-embed them as JSON when they parse, else as text.
fn passthrough(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// Run the backbone until the process is killed.
pub fn serve(cfg: Config, addr: &str) -> Result<(), String> {
    let server = tiny_http::Server::http(addr).map_err(|e| e.to_string())?;
    let auth_note = if cfg.token.is_some() {
        "token required"
    } else {
        "OPEN (no token)"
    };
    println!(
        "Marrow backbone on http://{addr}  (data: {}, {auth_note})",
        cfg.data_dir.display()
    );
    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        let path = url.split('?').next().unwrap_or("/").to_string();
        let auth = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .map(|h| h.value.as_str().to_string());
        let mut body = String::new();
        let _ = std::io::Read::read_to_string(request.as_reader(), &mut body);
        let r = handle(&cfg, &method, &path, auth.as_deref(), &body);
        let header =
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
        let resp = tiny_http::Response::from_string(r.body)
            .with_status_code(r.status)
            .with_header(header);
        let _ = request.respond(resp);
    }
    Ok(())
}

/// Resolve config from CLI-style overrides falling back to env then defaults.
pub fn config_from(data_dir: Option<PathBuf>, token: Option<String>) -> Config {
    let data_dir = data_dir
        .or_else(|| std::env::var_os("MARROW_DATA").map(PathBuf::from))
        .unwrap_or_else(|| Path::new("marrow-data").to_path_buf());
    let token = token
        .or_else(|| std::env::var("MARROW_TOKEN").ok())
        .filter(|t| !t.is_empty());
    Config { data_dir, token }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(token: Option<&str>) -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (
            Config {
                data_dir: dir.path().to_path_buf(),
                token: token.map(String::from),
            },
            dir,
        )
    }

    #[test]
    fn health_is_open_and_rpc_round_trips() {
        let (c, _d) = cfg(None);
        let h = handle(&c, "GET", "/health", None, "");
        assert_eq!(h.status, 200);

        let write = handle(
            &c,
            "POST",
            "/v1/rpc",
            None,
            r#"{"tool":"mem_write","project":"team","args":{"kind":"decision","topic":"auth","body":"Use JWT."}}"#,
        );
        assert_eq!(write.status, 200);
        assert!(write.body.contains("\"ok\":true"));

        let search = handle(
            &c,
            "POST",
            "/v1/rpc",
            None,
            r#"{"tool":"mem_search","project":"team","args":{"text":"JWT"}}"#,
        );
        assert!(search.body.contains("Use JWT"), "{}", search.body);
    }

    #[test]
    fn token_guards_everything_but_health() {
        let (c, _d) = cfg(Some("s3cret"));
        assert_eq!(handle(&c, "GET", "/health", None, "").status, 200);
        assert_eq!(handle(&c, "POST", "/v1/rpc", None, "{}").status, 401);
        assert_eq!(
            handle(
                &c,
                "POST",
                "/v1/rpc",
                Some("Bearer s3cret"),
                r#"{"tool":"mem_status"}"#
            )
            .status,
            200
        );
        assert_eq!(
            handle(&c, "POST", "/v1/rpc", Some("Bearer wrong"), "{}").status,
            401
        );
    }

    #[test]
    fn project_names_cannot_escape_the_data_dir() {
        assert!(safe_project("../etc").is_none());
        assert!(safe_project(".hidden").is_none());
        assert_eq!(safe_project("").unwrap(), "default");
        assert_eq!(safe_project("team-app.1").unwrap(), "team-app.1");
    }
}
