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

/// Forward one tool call to the backbone and return its result text (or an error message).
pub fn forward(name: &str, args: &Value) -> Result<String, String> {
    let base = endpoint().ok_or("MARROW_REMOTE not set")?;
    let project = std::env::var("MARROW_PROJECT").unwrap_or_else(|_| "default".into());
    let mut req = ureq::post(&format!("{base}/v1/rpc"));
    if let Ok(token) = std::env::var("MARROW_TOKEN") {
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
    }
    let resp = req
        .send_json(json!({"tool": name, "args": args, "project": project}))
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
