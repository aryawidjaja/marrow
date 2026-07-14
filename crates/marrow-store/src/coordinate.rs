//! The coordination plane: a real-time activity stream and advisory work-claims that let many
//! agent sessions share one brain instead of working blind to each other.
//!
//! Everything here rides the append-only episodic ledger — concurrent agents *append* claim,
//! release, and activity events; active claims are reconstructed by folding the stream. There
//! are no mutable shared records to contend on, so this is safe for many writers, and every
//! action lands in the same tamper-evident log (coordination is audit for free).

use std::path::Path;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use marrow_episodic::Event;
use marrow_memdocs::{Memory, MemoryKind, Status};

use crate::query::Query;
use crate::store::{Error, Store};
use crate::util;

/// The slice of work an agent is claiming, so others can tell if they would collide.
///
/// Matching is intentionally simple and *advisory*: a shared feature or topic, or an
/// overlapping file or symbol, counts as an overlap. File entries support a trailing `*`
/// wildcard (e.g. `src/auth/*`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaimScope {
    /// File paths or `dir/*` globs this work touches.
    #[serde(default)]
    pub files: Vec<String>,
    /// Code symbols (functions, types) this work touches.
    #[serde(default)]
    pub symbols: Vec<String>,
    /// A free-text topic the work concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// A named feature/task the work belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// The project this claim is scoped to.
    #[serde(default)]
    pub project_id: String,
}

impl ClaimScope {
    /// True if this scope and `other` plausibly touch the same work (advisory).
    pub fn overlaps(&self, other: &ClaimScope) -> bool {
        if !self.project_id.is_empty()
            && !other.project_id.is_empty()
            && self.project_id != other.project_id
        {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.feature, &other.feature) {
            if a == b {
                return true;
            }
        }
        if let (Some(a), Some(b)) = (&self.topic, &other.topic) {
            if a == b {
                return true;
            }
        }
        if self
            .files
            .iter()
            .any(|a| other.files.iter().any(|b| files_overlap(a, b)))
        {
            return true;
        }
        self.symbols.iter().any(|s| other.symbols.contains(s))
    }
}

fn files_overlap(a: &str, b: &str) -> bool {
    a == b || glob_match(a, b) || glob_match(b, a)
}

/// A minimal glob: `prefix*` matches any path starting with `prefix`.
fn glob_match(pattern: &str, path: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => path.starts_with(prefix),
        None => false,
    }
}

/// An advisory lease: "I'm working on this." TTL'd, so a dead session never deadlocks a repo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    /// Unique claim id (ULID).
    pub id: String,
    /// The session that holds the claim.
    pub session_id: String,
    /// Who registered it.
    pub actor: String,
    /// What is being claimed.
    pub scope: ClaimScope,
    /// Human-readable intent ("refactor auth to async").
    pub intent: String,
    /// When it was created (RFC3339).
    pub created_at: String,
    /// When the lease expires (RFC3339).
    pub expires_at: String,
    /// Lease length in seconds — used to extend the lease when the holder makes progress.
    #[serde(default)]
    pub ttl_secs: i64,
}

/// A warm-start briefing handed to a new session so it doesn't cold-start or re-scan.
#[derive(Debug, Clone)]
pub struct Briefing {
    /// The goal the session bootstrapped with.
    pub goal: String,
    /// What other sessions are currently working on.
    pub active_claims: Vec<Claim>,
    /// Memories relevant to the goal (under a token budget).
    pub relevant: Vec<Memory>,
    /// The most recent active decisions in the project.
    pub recent_decisions: Vec<Memory>,
    /// Set when the brain is empty but the repo has knowledge docs on disk — a hint to onboard the
    /// project with `marrow ingest` so future sessions start warm.
    pub suggest_ingest: bool,
    /// Set when enough new memories have piled up since the last cleanup — a hint to run
    /// consolidation so the brain stays coherent.
    pub suggest_consolidate: bool,
}

/// Project knowledge docs worth seeding memory from: root-level Markdown plus everything under
/// `docs/`, excluding VCS/build/Marrow dirs. Returns `(path relative to root, size in bytes)`,
/// sorted by path and capped, so an existing repo can be onboarded (`marrow ingest`) and a
/// cold-start session can be pointed at them.
pub fn knowledge_docs(root: &Path) -> Vec<(String, u64)> {
    const CAP: usize = 50;
    let mut out: Vec<(String, u64)> = Vec::new();

    // Root-level Markdown only (don't recurse the whole repo).
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if let Ok(md) = e.metadata() {
                if md.is_file() && is_markdown(&e.path()) {
                    out.push((rel_to(root, &e.path()), md.len()));
                }
            }
        }
    }
    // Everything under docs/ (recursive).
    collect_markdown(&root.join("docs"), root, &mut out);

    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out.truncate(CAP);
    out
}

fn is_markdown(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn rel_to(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_markdown(dir: &Path, root: &Path, out: &mut Vec<(String, u64)>) {
    const SKIP: &[&str] = &[".git", "target", "node_modules", ".marrow"];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(md) = e.metadata() else { continue };
        if md.is_dir() {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || SKIP.contains(&name) {
                continue;
            }
            collect_markdown(&p, root, out);
        } else if is_markdown(&p) {
            out.push((rel_to(root, &p), md.len()));
        }
    }
}

impl Store {
    /// Register an advisory work-claim. Returns the created claim (with id and expiry).
    pub fn claim(
        &self,
        session_id: &str,
        actor: &str,
        scope: ClaimScope,
        intent: &str,
        ttl_secs: i64,
    ) -> Result<Claim, Error> {
        let ttl = ttl_secs.max(0);
        let claim = Claim {
            id: Ulid::new().to_string(),
            session_id: session_id.to_string(),
            actor: actor.to_string(),
            scope,
            intent: intent.to_string(),
            created_at: util::now_rfc3339(),
            expires_at: util::rfc3339_after(ttl),
            ttl_secs: ttl,
        };
        let data = serde_json::to_value(&claim).map_err(|e| Error::Parse(e.to_string()))?;
        self.log_data("claim", actor, &format!("claim: {intent}"), data)?;
        Ok(claim)
    }

    /// Release a claim early (otherwise it expires at its TTL).
    pub fn release(&self, claim_id: &str, actor: &str) -> Result<(), Error> {
        self.log_data(
            "release",
            actor,
            &format!("released {claim_id}"),
            serde_json::json!({ "claim_id": claim_id }),
        )
    }

    /// All claims that are still active: registered, not released, renewed-or-not-expired.
    ///
    /// Folds the ledger: a `claim` registers a lease, a `renew` extends its expiry (so active
    /// work keeps its claim alive), and a `release` drops it. The latest renewal wins.
    pub fn active_claims(&self) -> Result<Vec<Claim>, Error> {
        use std::collections::{HashMap, HashSet};
        let now = util::to_unix(&util::now_rfc3339()).unwrap_or(0);
        let mut by_id: HashMap<String, Claim> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut renewed: HashMap<String, String> = HashMap::new();
        let mut released: HashSet<String> = HashSet::new();
        for ev in self.history()? {
            match ev.kind.as_str() {
                "claim" => {
                    if let Ok(c) = serde_json::from_value::<Claim>(ev.data.clone()) {
                        if !by_id.contains_key(&c.id) {
                            order.push(c.id.clone());
                        }
                        by_id.insert(c.id.clone(), c);
                    }
                }
                "renew" => {
                    if let (Some(id), Some(exp)) = (
                        ev.data.get("claim_id").and_then(|v| v.as_str()),
                        ev.data.get("expires_at").and_then(|v| v.as_str()),
                    ) {
                        renewed.insert(id.to_string(), exp.to_string());
                    }
                }
                "release" => {
                    if let Some(id) = ev.data.get("claim_id").and_then(|v| v.as_str()) {
                        released.insert(id.to_string());
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for id in order {
            if released.contains(&id) {
                continue;
            }
            let Some(mut c) = by_id.remove(&id) else {
                continue;
            };
            if let Some(exp) = renewed.get(&id) {
                if util::to_unix(exp) >= util::to_unix(&c.expires_at) {
                    c.expires_at = exp.clone();
                }
            }
            if util::to_unix(&c.expires_at).is_some_and(|e| e > now) {
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Active claims that would collide with `scope` — what an agent checks before starting work.
    pub fn claims_overlapping(&self, scope: &ClaimScope) -> Result<Vec<Claim>, Error> {
        Ok(self
            .active_claims()?
            .into_iter()
            .filter(|c| c.scope.overlaps(scope))
            .collect())
    }

    /// Record a unit of progress so other sessions can see it in real time. Recording progress
    /// also **renews the lease** on this session's claims that cover the touched files, so a long
    /// task never loses its claim mid-work (claims can only otherwise expire at their TTL).
    pub fn progress(
        &self,
        session_id: &str,
        actor: &str,
        summary: &str,
        files: &[String],
    ) -> Result<(), Error> {
        self.log_data(
            "progress",
            actor,
            summary,
            serde_json::json!({ "session_id": session_id, "files": files }),
        )?;
        self.renew_claims(session_id, actor, files)
    }

    /// Extend the lease on this session's active claims that overlap any of `files`. A no-op if
    /// `files` is empty or nothing matches.
    fn renew_claims(&self, session_id: &str, actor: &str, files: &[String]) -> Result<(), Error> {
        if files.is_empty() {
            return Ok(());
        }
        let touched = ClaimScope {
            files: files.to_vec(),
            ..Default::default()
        };
        for c in self.active_claims()? {
            if c.session_id == session_id && c.scope.overlaps(&touched) {
                let ttl = if c.ttl_secs > 0 { c.ttl_secs } else { 900 };
                self.log_data(
                    "renew",
                    actor,
                    &format!("renew: {}", c.intent),
                    serde_json::json!({ "claim_id": c.id, "expires_at": util::rfc3339_after(ttl) }),
                )?;
            }
        }
        Ok(())
    }

    /// The most recent activity-stream events (newest first), capped at `limit`.
    pub fn activity(&self, limit: usize) -> Result<Vec<Event>, Error> {
        let mut all = self.history()?;
        all.reverse();
        all.truncate(limit);
        Ok(all)
    }

    /// Warm-start a new session: announce it, then return what others are doing plus the
    /// memories and decisions most relevant to `goal` — so the agent starts informed, not cold.
    pub fn bootstrap(
        &self,
        goal: &str,
        project_id: &str,
        actor: &str,
        max_tokens: usize,
    ) -> Result<Briefing, Error> {
        self.log_data(
            "session_started",
            actor,
            &format!("session started: {goal}"),
            serde_json::json!({ "goal": goal, "project_id": project_id }),
        )?;

        let active_claims = self
            .active_claims()?
            .into_iter()
            .filter(|c| c.scope.project_id.is_empty() || c.scope.project_id == project_id)
            .collect();

        // Pull memories relevant to the goal. A new session should never start cold, so if the
        // goal doesn't match anything (e.g. a vague natural-language goal with no embedder
        // configured), fall back to the most recent project memories under the same budget.
        let budgeted = |text: Option<&str>| Query {
            project_id: Some(project_id.to_string()),
            exclude_expired: true,
            max_tokens: Some(max_tokens),
            hybrid_weight: text.map(|_| 1.0),
            ..Query::default()
        };
        let mut relevant = self.recall(goal, &budgeted(Some(goal)), actor)?;
        if relevant.is_empty() {
            relevant = self.query(&budgeted(None))?;
        }

        let recent_decisions = self.query(&Query {
            project_id: Some(project_id.to_string()),
            kind: Some(MemoryKind::Decision),
            status: Some(Status::Active),
            exclude_expired: true,
            limit: Some(5),
            ..Query::default()
        })?;

        // Onboarding hint: an empty brain in a repo that already has docs should be seeded, so the
        // first session points the agent at `marrow ingest` instead of starting cold.
        let suggest_ingest = self.list().map(|r| r.is_empty()).unwrap_or(false)
            && !knowledge_docs(self.root()).is_empty();
        let suggest_consolidate = self.consolidation_due().unwrap_or(false);

        Ok(Briefing {
            goal: goal.to_string(),
            active_claims,
            relevant,
            recent_decisions,
            suggest_ingest,
            suggest_consolidate,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(files: &[&str], project: &str) -> ClaimScope {
        ClaimScope {
            files: files.iter().map(|s| s.to_string()).collect(),
            project_id: project.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn scopes_overlap_on_shared_file_and_glob() {
        let a = scope(&["src/auth.rs"], "p");
        let b = scope(&["src/auth.rs"], "p");
        assert!(a.overlaps(&b));

        let g = ClaimScope {
            files: vec!["src/auth/*".into()],
            project_id: "p".into(),
            ..Default::default()
        };
        assert!(g.overlaps(&scope(&["src/auth/login.rs"], "p")));
        assert!(!a.overlaps(&scope(&["src/billing.rs"], "p")));
    }

    #[test]
    fn scopes_overlap_on_feature_but_not_across_projects() {
        let a = ClaimScope {
            feature: Some("checkout".into()),
            project_id: "p".into(),
            ..Default::default()
        };
        let b = ClaimScope {
            feature: Some("checkout".into()),
            project_id: "p".into(),
            ..Default::default()
        };
        assert!(a.overlaps(&b));

        let other_project = ClaimScope {
            feature: Some("checkout".into()),
            project_id: "q".into(),
            ..Default::default()
        };
        assert!(!a.overlaps(&other_project));
    }

    #[test]
    fn claim_is_active_then_released() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let c = store
            .claim(
                "sess-1",
                "agent-a",
                scope(&["src/auth.rs"], "demo"),
                "refactor auth",
                1800,
            )
            .unwrap();
        assert_eq!(store.active_claims().unwrap().len(), 1);

        // A second agent checks before working on the same file and sees the collision.
        let conflict = store
            .claims_overlapping(&scope(&["src/auth.rs"], "demo"))
            .unwrap();
        assert_eq!(conflict.len(), 1);
        assert_eq!(conflict[0].intent, "refactor auth");

        store.release(&c.id, "agent-a").unwrap();
        assert!(store.active_claims().unwrap().is_empty());
    }

    #[test]
    fn expired_claims_are_not_active() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        // TTL of 0 → expires immediately.
        store
            .claim("sess-1", "agent-a", scope(&["src/x.rs"], "demo"), "x", 0)
            .unwrap();
        assert!(store.active_claims().unwrap().is_empty());
    }

    #[test]
    fn activity_returns_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .claim("s", "a", scope(&["f1"], "demo"), "first", 600)
            .unwrap();
        // progress on an unclaimed file → no lease renewal, so the order stays claim, progress.
        store
            .progress("s", "a", "did a thing", &["f2".into()])
            .unwrap();
        let recent = store.activity(10).unwrap();
        assert_eq!(recent[0].kind, "progress");
        assert_eq!(recent[1].kind, "claim");
    }

    #[test]
    fn progress_renews_the_holders_lease() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let c = store
            .claim("s1", "a", scope(&["src/auth.rs"], "demo"), "auth", 3600)
            .unwrap();
        store
            .progress("s1", "a", "edited auth", &["src/auth.rs".into()])
            .unwrap();
        // A renewal event was recorded, and the claim is still a single active claim (deduped).
        assert!(store.history().unwrap().iter().any(|e| e.kind == "renew"));
        let active = store.active_claims().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, c.id);
    }

    #[test]
    fn progress_does_not_renew_another_sessions_lease() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .claim("owner", "a", scope(&["src/x.rs"], "demo"), "x", 3600)
            .unwrap();
        store
            .progress("intruder", "b", "poking", &["src/x.rs".into()])
            .unwrap();
        assert!(
            !store.history().unwrap().iter().any(|e| e.kind == "renew"),
            "another session's progress must not renew the owner's lease"
        );
    }

    #[test]
    fn bootstrap_briefs_with_claims_and_relevant_memory() {
        use marrow_memdocs::{Frontmatter, Provenance, Scope};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let mut m = Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Decision,
                status: Status::Active,
                topic: Some("auth".into()),
                area: None,
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "test".into(),
                    model: None,
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: "We use short-lived JWTs for auth sessions.".into(),
        };
        store.write(&mut m).unwrap();
        store
            .claim(
                "other",
                "agent-b",
                scope(&["src/auth.rs"], "demo"),
                "auth work",
                600,
            )
            .unwrap();

        let brief = store
            .bootstrap("how does auth work", "demo", "agent-a", 1000)
            .unwrap();
        assert_eq!(brief.active_claims.len(), 1);
        assert_eq!(brief.recent_decisions.len(), 1);
        assert!(!brief.relevant.is_empty());
        // bootstrap announced the session in the ledger.
        assert!(store
            .history()
            .unwrap()
            .iter()
            .any(|e| e.kind == "session_started"));
    }

    #[test]
    fn knowledge_docs_finds_root_and_docs_excludes_build() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("README.md"), "readme").unwrap();
        std::fs::write(root.join("notes.txt"), "not markdown").unwrap();
        std::fs::create_dir_all(root.join("docs/sub")).unwrap();
        std::fs::write(root.join("docs/guide.md"), "guide").unwrap();
        std::fs::write(root.join("docs/sub/deep.md"), "deep").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/build.md"), "build artifact").unwrap();
        std::fs::create_dir_all(root.join("docs/node_modules")).unwrap();
        std::fs::write(root.join("docs/node_modules/dep.md"), "dep").unwrap();

        let found: Vec<String> = knowledge_docs(root).into_iter().map(|(p, _)| p).collect();
        assert_eq!(
            found,
            vec![
                "README.md".to_string(),
                "docs/guide.md".to_string(),
                "docs/sub/deep.md".to_string(),
            ]
        );
    }

    #[test]
    fn bootstrap_suggests_ingest_only_when_empty_with_docs() {
        use marrow_memdocs::{Frontmatter, Provenance, Scope};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        // No docs, empty brain → no hint.
        assert!(
            !store
                .bootstrap("resume", "demo", "a", 1000)
                .unwrap()
                .suggest_ingest
        );

        // Empty brain + a doc on disk → hint.
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        assert!(
            store
                .bootstrap("resume", "demo", "a", 1000)
                .unwrap()
                .suggest_ingest
        );

        // Once the brain has a memory, stop nudging even though the doc is still there.
        let mut m = Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: Some("t".into()),
                area: None,
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "test".into(),
                    model: None,
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: "a memory".into(),
        };
        store.write(&mut m).unwrap();
        assert!(
            !store
                .bootstrap("resume", "demo", "a", 1000)
                .unwrap()
                .suggest_ingest
        );
    }
}
