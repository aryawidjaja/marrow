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
        if !secure_gateway(&self.url) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "gateway must use https; plain http is allowed only on localhost",
            ));
        }
        let path = Self::path(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self).unwrap_or_default())?;
        // A sharing token is a credential. Keep it out of Git even in projects that intentionally
        // version other `.marrow` documents, and make the file owner-only on Unix.
        let ignore = root.join(".gitignore");
        let mut text = std::fs::read_to_string(&ignore).unwrap_or_default();
        if !text
            .lines()
            .any(|line| line.trim() == ".marrow/remote.toml")
        {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(".marrow/remote.toml\n");
            std::fs::write(ignore, text)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
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

fn secure_gateway(url: &str) -> bool {
    url.starts_with("https://")
        || ["http://127.0.0.1", "http://localhost", "http://[::1]"]
            .iter()
            .any(|local| url == *local || url.starts_with(&format!("{local}:")))
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
        assert!(std::fs::read_to_string(root.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == ".marrow/remote.toml"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(SharedRemote::path(root))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        assert!(SharedRemote::remove(root).unwrap(), "was shared");
        assert!(SharedRemote::load(root).is_none(), "local again");
        assert!(!SharedRemote::remove(root).unwrap(), "nothing to remove");
    }

    #[test]
    fn remote_gateways_require_tls_off_machine() {
        let dir = tempfile::tempdir().unwrap();
        let remote = SharedRemote {
            url: "http://example.com".into(),
            space: "app".into(),
            token: None,
        };
        assert_eq!(
            remote.save(dir.path()).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(SharedRemote {
            url: "http://127.0.0.1:8787".into(),
            ..remote
        }
        .save(dir.path())
        .is_ok());
    }
}
