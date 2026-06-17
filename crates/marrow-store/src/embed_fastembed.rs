//! Local ONNX embedding backend via fastembed (feature `embed-fastembed`).
//!
//! Runs fully offline after the model is fetched once. The default model is multilingual
//! (Multilingual-E5-small), so non-English memories — including Arabic — embed sensibly.

use std::sync::Mutex;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

use crate::config::EmbeddingConfig;
use crate::embed::{EmbedError, Embedder};

pub struct FastEmbedder {
    model: Mutex<TextEmbedding>,
    dim: usize,
}

impl FastEmbedder {
    /// Build from config. Returns None if the model fails to initialize.
    pub fn from_config(cfg: &EmbeddingConfig) -> Option<FastEmbedder> {
        let model = match cfg.model.as_str() {
            "bge-small" | "bge-small-en" => EmbeddingModel::BGESmallENV15,
            "minilm" => EmbeddingModel::AllMiniLML6V2,
            "multilingual-e5-large" | "e5-large" => EmbeddingModel::MultilingualE5Large,
            // Default is multilingual so Arabic and other non-English text embed well.
            _ => EmbeddingModel::MultilingualE5Small,
        };
        let embedding = TextEmbedding::try_new(TextInitOptions::new(model)).ok()?;
        Some(FastEmbedder {
            model: Mutex::new(embedding),
            dim: cfg.dim,
        })
    }
}

impl Embedder for FastEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let docs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.model
            .lock()
            .map_err(|e| EmbedError::Backend(e.to_string()))?
            .embed(docs, None)
            .map_err(|e| EmbedError::Backend(e.to_string()))
    }
}
