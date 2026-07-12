//! Per-project sharing. A project can be pointed at a gateway "space" so its memory lives there and
//! is shared with other machines or teammates, while every other project stays local and private.
//! The mark lives in the project's own `.marrow/remote.toml`, so routing is decided per project.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a shared project's memory actually lives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedRemote {
    /// Gateway base URL, e.g. `https://team.fly.dev`.
    pub url: String,
    /// The namespaced space on the gateway (the shared brain's key). Machines that use the same
    /// url + space + token share one brain.
    pub space: String,
    /// Bearer token / API key for the gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl SharedRemote {
    fn path(root: &Path) -> PathBuf {
        root.join(".marrow").join("remote.toml")
    }

    /// The project's sharing config, if it has been shared.
    pub fn load(root: &Path) -> Option<Self> {
        std::fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
    }

    /// Mark this project as shared (writes the config).
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self).unwrap_or_default())
    }

    /// Make this project local again (removes the config). Returns whether it was shared.
    pub fn remove(root: &Path) -> std::io::Result<bool> {
        let path = Self::path(root);
        if path.exists() {
            std::fs::remove_file(path)?;
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_remove_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(SharedRemote::load(root).is_none(), "unshared by default");

        let remote = SharedRemote {
            url: "https://team.fly.dev".into(),
            space: "app".into(),
            token: Some("secret".into()),
        };
        remote.save(root).unwrap();
        assert_eq!(SharedRemote::load(root).as_ref(), Some(&remote));

        assert!(SharedRemote::remove(root).unwrap(), "was shared");
        assert!(SharedRemote::load(root).is_none(), "local again");
        assert!(!SharedRemote::remove(root).unwrap(), "nothing to remove");
    }
}
