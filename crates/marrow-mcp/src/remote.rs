//! Remote mode: when `MARROW_REMOTE` is set, tool calls go to a shared Marrow backbone over HTTP
//! instead of a local store, so agents on different machines share one brain. The transport is the
//! same `{tool, args, project}` envelope the backbone speaks (see marrow-server).

use serde_json::{json, Value};

/// The backbone URL, if this agent is wired to one.
pub fn endpoint() -> Option<String> {
    std::env::var("MARROW_REMOTE")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

/// Forward one tool call using the env-configured backbone (`MARROW_REMOTE`/`MARROW_TOKEN`/
/// `MARROW_PROJECT`). This is the machine-wide "everything remote" mode.
pub fn forward(name: &str, args: &Value) -> Result<String, String> {
    let base = endpoint().ok_or("MARROW_REMOTE not set")?;
    let project = std::env::var("MARROW_PROJECT").unwrap_or_else(|_| "default".into());
    let token = std::env::var("MARROW_TOKEN").ok().filter(|t| !t.is_empty());
    forward_to(&base, token.as_deref(), &project, name, args)
}

/// Forward one tool call to a specific gateway space and return its result text (or an error
/// message). This is what a shared project routes to, so each project can target its own space.
pub fn forward_to(
    url: &str,
    token: Option<&str>,
    space: &str,
    name: &str,
    args: &Value,
) -> Result<String, String> {
    let base = url.trim_end_matches('/');
    let mut req = ureq::post(&format!("{base}/v1/rpc"));
    if let Some(token) = token {
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
    }
    let resp = req
        .send_json(json!({"tool": name, "args": args, "project": space}))
        .map_err(|e| format!("marrow backbone unreachable: {e}"))?;
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("bad backbone response: {e}"))?;
    if body.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(match body.get("result") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => "{}".to_string(),
        })
    } else {
        Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("backbone error")
            .to_string())
    }
}
