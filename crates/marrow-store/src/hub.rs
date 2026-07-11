//! The Marrow hub: an opt-in overlay that federates the per-project brains on one machine into a
//! single second brain — cross-project recall, cross-session awareness, and a shared `core` store
//! (memory about the user) that every project can see.
//!
//! Projects stay sovereign: their markdown + index remain the source of truth and nothing is
//! copied or migrated. The hub only keeps a registry of project roots and its own core store, and
//! unions their results at read time. A project participates only once it is registered, so the
//! per-project privacy default is preserved — cross-project sharing is a deliberate opt-in.

use std::fs;
use std::path::{Path, PathBuf};

use marrow_episodic::Event;
use marrow_memdocs::Memory;
use serde::{Deserialize, Serialize};

use crate::query::Query;
use crate::store::{Error, Store};

/// A project registered with the hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    projects: Vec<Project>,
}

/// A memory found in the hive, tagged with the project it came from.
pub struct HubHit {
    pub project: String,
    pub memory: Memory,
}

/// An activity event tagged with the project it happened in.
pub struct HubEvent {
    pub project: String,
    pub event: Event,
}

/// The federation layer over the machine's registered project brains plus a shared core store.
pub struct Hub {
    home: PathBuf,
    registry: Registry,
}

impl Hub {
    /// The hub home: `$MARROW_HUB`, else `~/.marrow/hub`.
    pub fn home() -> PathBuf {
        if let Some(h) = std::env::var_os("MARROW_HUB") {
            return PathBuf::from(h);
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".marrow")
            .join("hub")
    }

    /// Open (creating if needed) the hub at its home directory.
    pub fn open() -> Result<Hub, Error> {
        let home = Self::home();
        fs::create_dir_all(&home).map_err(|e| Error::Io(e.to_string()))?;
        let registry = fs::read_to_string(home.join("registry.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Ok(Hub { home, registry })
    }

    fn save(&self) -> Result<(), Error> {
        let json =
            serde_json::to_string_pretty(&self.registry).map_err(|e| Error::Io(e.to_string()))?;
        fs::write(self.home.join("registry.json"), json).map_err(|e| Error::Io(e.to_string()))
    }

    /// The shared core store, where memory about the user (or any cross-project fact) lives. It is
    /// created on first use and is always included in federated reads.
    pub fn core(&self) -> Result<Store, Error> {
        let root = self.home.join("core");
        if !root.join(".marrow").is_dir() {
            Store::init(&root)?;
        }
        Store::open(&root)
    }

    /// Register a project so it joins the hive. Idempotent: re-registering a path updates its name.
    /// The name defaults to the directory basename.
    pub fn register(&mut self, root: &Path, name: Option<&str>) -> Result<Project, Error> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let name = name
            .map(str::to_string)
            .or_else(|| {
                root.file_name()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "project".into());
        let project = Project {
            name,
            root: root.clone(),
        };
        self.registry.projects.retain(|p| p.root != root);
        self.registry.projects.push(project.clone());
        self.save()?;
        Ok(project)
    }

    /// Drop a project from the hive by name or root path. Returns whether anything was removed.
    pub fn forget(&mut self, key: &str) -> Result<bool, Error> {
        let before = self.registry.projects.len();
        let key_path = Path::new(key).canonicalize().ok();
        self.registry
            .projects
            .retain(|p| p.name != key && Some(&p.root) != key_path.as_ref());
        let removed = self.registry.projects.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Registered projects whose store still exists on disk (dead roots are skipped, not pruned —
    /// a project on an unmounted drive should return once it is back).
    pub fn projects(&self) -> Vec<Project> {
        self.registry
            .projects
            .iter()
            .filter(|p| p.root.join(".marrow").is_dir())
            .cloned()
            .collect()
    }

    /// Open every live project store plus core, tagged by project name. Core is first so shared
    /// user memory always leads.
    fn stores(&self) -> Vec<(String, Store)> {
        let mut out = Vec::new();
        if let Ok(core) = self.core() {
            out.push(("core".to_string(), core));
        }
        for p in self.projects() {
            if let Ok(s) = Store::open(&p.root) {
                out.push((p.name.clone(), s));
            }
        }
        out
    }

    /// Cross-project recall: union goal-relevant memories from every registered brain, tagged by
    /// project. `per_project` bounds how many each brain contributes before the global cap.
    pub fn recall(&self, text: &str, per_project: usize, limit: usize) -> Vec<HubHit> {
        let query = Query {
            status: Some(marrow_memdocs::Status::Active),
            exclude_expired: true,
            limit: Some(per_project),
            hybrid_weight: Some(1.0),
            ..Query::default()
        };
        let mut hits = Vec::new();
        for (project, store) in self.stores() {
            let found = store
                .recall(text, &query, "hub")
                .or_else(|_| store.search(text, &query))
                .unwrap_or_default();
            for memory in found {
                hits.push(HubHit {
                    project: project.clone(),
                    memory,
                });
            }
        }
        hits.truncate(limit);
        hits
    }

    /// Cross-project awareness: the most recent activity across every registered brain, newest
    /// first — what agents in *other* projects are doing right now.
    pub fn activity(&self, limit: usize) -> Vec<HubEvent> {
        let mut events = Vec::new();
        for (project, store) in self.stores() {
            if let Ok(evts) = store.activity(limit) {
                for event in evts {
                    events.push(HubEvent {
                        project: project.clone(),
                        event,
                    });
                }
            }
        }
        events.sort_by(|a, b| b.event.ts.cmp(&a.event.ts));
        events.truncate(limit);
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope, Status};

    fn fact(topic: &str, body: &str) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: Some(topic.into()),
                scope: Scope {
                    user_id: None,
                    agent_id: None,
                    project_id: String::new(),
                    org_id: None,
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "test".into(),
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: body.into(),
        }
    }

    fn isolated_hub() -> (Hub, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("MARROW_HUB", dir.path().join("hub"));
        (Hub::open().unwrap(), dir)
    }

    #[test]
    fn federates_recall_and_activity_across_projects() {
        // Serialize the env-var mutation these tests share.
        let _guard = TEST_LOCK.lock().unwrap();
        let (mut hub, dir) = isolated_hub();

        let proj_a = dir.path().join("a");
        let proj_b = dir.path().join("b");
        let a = Store::init(&proj_a).unwrap();
        let b = Store::init(&proj_b).unwrap();
        a.write(&mut fact("auth", "Project A uses JWT rotation for auth."))
            .unwrap();
        b.write(&mut fact(
            "billing",
            "Project B charges via Stripe with JWT-signed webhooks.",
        ))
        .unwrap();

        hub.register(&proj_a, Some("alpha")).unwrap();
        hub.register(&proj_b, Some("beta")).unwrap();

        let hits = hub.recall("JWT", 5, 10);
        let projects: Vec<&str> = hits.iter().map(|h| h.project.as_str()).collect();
        assert!(projects.contains(&"alpha"), "got {projects:?}");
        assert!(projects.contains(&"beta"), "got {projects:?}");

        let act = hub.activity(20);
        assert!(act.iter().any(|e| e.project == "alpha"));
        assert!(act.iter().any(|e| e.project == "beta"));
        std::env::remove_var("MARROW_HUB");
    }

    #[test]
    fn register_is_idempotent_and_forget_removes() {
        let _guard = TEST_LOCK.lock().unwrap();
        let (mut hub, dir) = isolated_hub();
        let proj = dir.path().join("p");
        Store::init(&proj).unwrap();
        hub.register(&proj, Some("p1")).unwrap();
        hub.register(&proj, Some("p1")).unwrap();
        assert_eq!(hub.projects().len(), 1);
        assert!(hub.forget("p1").unwrap());
        assert_eq!(hub.projects().len(), 0);
        std::env::remove_var("MARROW_HUB");
    }

    use std::sync::Mutex;
    static TEST_LOCK: Mutex<()> = Mutex::new(());
}
