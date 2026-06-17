//! OpenAI-compatible HTTP embedding backend (feature `embed-http`).
//!
//! Works with any endpoint that accepts `{"model", "input": [..]}` and returns
//! `{"data": [{"embedding": [..]}]}`. The API key is read from `MARROW_EMBED_API_KEY`,
//! never from config or disk.

use serde_json::json;

use crate::config::EmbeddingConfig;
use crate::embed::{EmbedError, Embedder};

pub struct HttpEmbedder {
    url: String,
    model: String,
    dim: usize,
    api_key: Option<String>,
}

impl HttpEmbedder {
    pub fn from_config(cfg: &EmbeddingConfig) -> Self {
        HttpEmbedder {
            url: cfg.url.clone(),
            model: cfg.model.clone(),
            dim: cfg.dim,
            api_key: std::env::var("MARROW_EMBED_API_KEY").ok(),
        }
    }
}

impl Embedder for HttpEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut req = ureq::post(&self.url).set("content-type", "application/json");
        if let Some(key) = &self.api_key {
            req = req.set("authorization", &format!("Bearer {key}"));
        }
        let resp = req
            .send_json(json!({"model": self.model, "input": texts}))
            .map_err(|e| EmbedError::Http(e.to_string()))?;
        let value: serde_json::Value = resp
            .into_json()
            .map_err(|e| EmbedError::Http(e.to_string()))?;
        parse_embeddings(&value)
    }
}

/// Parse an OpenAI-style `{data:[{embedding:[...]}]}` response.
pub fn parse_embeddings(value: &serde_json::Value) -> Result<Vec<Vec<f32>>, EmbedError> {
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| EmbedError::Shape("missing data array".into()))?;
    let mut out = Vec::with_capacity(data.len());
    for item in data {
        let arr = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| EmbedError::Shape("missing embedding array".into()))?;
        out.push(
            arr.iter()
                .filter_map(|x| x.as_f64().map(|f| f as f32))
                .collect(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openai_shape() {
        let v = json!({"data": [{"embedding": [0.1, 0.2, 0.3]}, {"embedding": [1.0, 2.0]}]});
        let out = parse_embeddings(&v).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 3);
        assert_eq!(out[1], vec![1.0, 2.0]);
    }

    #[test]
    fn rejects_bad_shape() {
        assert!(parse_embeddings(&json!({"nope": true})).is_err());
    }
}
