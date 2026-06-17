//! Store configuration persisted as `.marrow/.marrow.toml`.

use serde::{Deserialize, Serialize};

/// Per-store configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Default project id stamped onto memories that don't specify one.
    pub project_id: String,
    /// Whether to HMAC-sign entries on write (enterprise integrity).
    #[serde(default)]
    pub sign: bool,
    /// Default embedding/semantic settings reserved for future use.
    #[serde(default)]
    pub semantic: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            project_id: "default".to_string(),
            sign: false,
            semantic: false,
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
            semantic: false,
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
}
