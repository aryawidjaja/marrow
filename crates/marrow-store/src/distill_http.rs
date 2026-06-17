//! LLM-backed distiller over an OpenAI-compatible chat endpoint (feature `distill-http`).
//!
//! Point `distiller_url` at a local or sovereign-hosted model (e.g. Core42, a self-hosted
//! vLLM/Ollama gateway) so consolidation can resolve genuine contradictions without any data
//! leaving your infrastructure. The API key, if any, comes from `MARROW_DISTILL_API_KEY`.

use serde_json::json;

use crate::config::ConsolidationConfig;
use crate::consolidate::{ClusterAction, Distiller, Verdict};

const SYSTEM_PROMPT: &str = "You consolidate an AI agent's memory. You are given several \
notes that are about the same topic. Decide one of: MERGE if they say the same thing, \
CONFLICT if they contradict each other, KEEP if they are actually distinct. Reply with ONLY \
a JSON object: {\"action\":\"merge|conflict|keep\",\"body\":\"the single distilled note that \
best represents the truth\",\"rationale\":\"one short sentence\"}.";

pub struct HttpDistiller {
    url: String,
    model: String,
    api_key: Option<String>,
}

impl HttpDistiller {
    pub fn from_config(cfg: &ConsolidationConfig) -> Self {
        HttpDistiller {
            url: cfg.distiller_url.clone(),
            model: cfg.distiller_model.clone(),
            api_key: std::env::var("MARROW_DISTILL_API_KEY").ok(),
        }
    }
}

impl Distiller for HttpDistiller {
    fn distill(&self, bodies: &[String]) -> Result<Verdict, String> {
        let listed = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| format!("{}. {}", i + 1, b.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        let mut req = ureq::post(&self.url).set("content-type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.set("authorization", &format!("Bearer {key}"));
        }
        let payload = json!({
            "model": self.model,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": listed},
            ],
        });
        let resp = req.send_json(payload).map_err(|e| e.to_string())?;
        let value: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        let content = chat_content(&value).ok_or_else(|| "no content in response".to_string())?;
        Ok(parse_verdict(&content))
    }
}

/// Extract `choices[0].message.content` from a chat-completions response.
pub fn chat_content(value: &serde_json::Value) -> Option<String> {
    value
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)
}

/// Parse the model's JSON reply into a [`Verdict`]. Anything unparseable is treated as `Keep`,
/// so a misbehaving model never destroys or merges memories by accident.
pub fn parse_verdict(content: &str) -> Verdict {
    let parsed: serde_json::Value = match serde_json::from_str(content.trim()) {
        Ok(v) => v,
        Err(_) => return keep(),
    };
    let action = match parsed.get("action").and_then(|a| a.as_str()) {
        Some("merge") => ClusterAction::Merge,
        Some("conflict") => ClusterAction::Conflict,
        _ => return keep(),
    };
    Verdict {
        action,
        body: parsed
            .get("body")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string(),
        rationale: parsed
            .get("rationale")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string(),
    }
}

fn keep() -> Verdict {
    Verdict {
        action: ClusterAction::Keep,
        body: String::new(),
        rationale: "left distinct".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_chat_content() {
        let v = json!({"choices": [{"message": {"content": "hello"}}]});
        assert_eq!(chat_content(&v).as_deref(), Some("hello"));
    }

    #[test]
    fn parses_a_conflict_verdict() {
        let v =
            parse_verdict(r#"{"action":"conflict","body":"the truth","rationale":"newer wins"}"#);
        assert_eq!(v.action, ClusterAction::Conflict);
        assert_eq!(v.body, "the truth");
    }

    #[test]
    fn parses_a_merge_verdict() {
        let v = parse_verdict(r#"{"action":"merge","body":"one note"}"#);
        assert_eq!(v.action, ClusterAction::Merge);
    }

    #[test]
    fn garbage_is_safely_kept() {
        assert_eq!(parse_verdict("not json").action, ClusterAction::Keep);
        assert_eq!(
            parse_verdict(r#"{"action":"explode"}"#).action,
            ClusterAction::Keep
        );
    }
}
