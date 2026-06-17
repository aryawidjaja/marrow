//! Store configuration persisted as `.marrow/.marrow.toml`.

use serde::{Deserialize, Serialize};

/// Embedding backend configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Backend: "none" | "hash" | "http" | "fastembed".
    pub provider: String,
    /// Model name (backend-specific).
    pub model: String,
    /// Endpoint for the "http" provider.
    pub url: String,
    /// Embedding dimension.
    pub dim: usize,
    /// Default hybrid weight (0 = keyword only, 1 = semantic only).
    pub default_weight: f64,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        EmbeddingConfig {
            provider: "none".to_string(),
            model: "bge-m3".to_string(),
            url: "http://localhost:8080/v1/embeddings".to_string(),
            dim: 256,
            default_weight: 0.5,
        }
    }
}

/// Per-store configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Default project id stamped onto memories that don't specify one.
    pub project_id: String,
    /// Whether to HMAC-sign entries on write (enterprise integrity).
    #[serde(default)]
    pub sign: bool,
    /// Embedding backend for semantic search.
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            project_id: "default".to_string(),
            sign: false,
            embedding: EmbeddingConfig::default(),
        }
    }
}

impl Config {
    /// Parse from TOML text.
    pub fn from_toml(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// Serialize to TOML text.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_toml() {
        let c = Config {
            project_id: "demo".into(),
            sign: true,
            embedding: EmbeddingConfig {
                provider: "http".into(),
                ..EmbeddingConfig::default()
            },
        };
        let parsed = Config::from_toml(&c.to_toml()).unwrap();
        assert_eq!(c, parsed);
    }

    #[test]
    fn defaults_fill_missing_fields() {
        let c = Config::from_toml("project_id = \"x\"\n").unwrap();
        assert_eq!(c.project_id, "x");
        assert!(!c.sign);
    }

    #[test]
    fn embedding_config_defaults() {
        let c = Config::from_toml("project_id = \"x\"\n").unwrap();
        assert_eq!(c.embedding.provider, "none");
        assert_eq!(c.embedding.default_weight, 0.5);
    }
}
