//! The coordination plane: a real-time activity stream and advisory work-claims that let many
//! agent sessions share one brain instead of working blind to each other.
//!
//! Everything here rides the append-only episodic ledger — concurrent agents *append* claim,
//! release, and activity events; active claims are reconstructed by folding the stream. There
//! are no mutable shared records to contend on, so this is safe for many writers, and every
//! action lands in the same tamper-evident log (coordination is audit for free).

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
        let claim = Claim {
            id: Ulid::new().to_string(),
            session_id: session_id.to_string(),
            actor: actor.to_string(),
            scope,
            intent: intent.to_string(),
            created_at: util::now_rfc3339(),
            expires_at: util::rfc3339_after(ttl_secs.max(0)),
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

    /// All claims that are still active: registered, not released, not expired.
    pub fn active_claims(&self) -> Result<Vec<Claim>, Error> {
        let now = util::to_unix(&util::now_rfc3339()).unwrap_or(0);
        let mut claims: Vec<Claim> = Vec::new();
        let mut released: std::collections::HashSet<String> = std::collections::HashSet::new();
        for ev in self.history()? {
            match ev.kind.as_str() {
                "claim" => {
                    if let Ok(c) = serde_json::from_value::<Claim>(ev.data.clone()) {
                        claims.push(c);
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
        Ok(claims
            .into_iter()
            .filter(|c| !released.contains(&c.id))
            .filter(|c| util::to_unix(&c.expires_at).is_some_and(|e| e > now))
            .collect())
    }

    /// Active claims that would collide with `scope` — what an agent checks before starting work.
    pub fn claims_overlapping(&self, scope: &ClaimScope) -> Result<Vec<Claim>, Error> {
        Ok(self
            .active_claims()?
            .into_iter()
            .filter(|c| c.scope.overlaps(scope))
            .collect())
    }

    /// Record a unit of progress so other sessions can see it in real time.
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
        )
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

        Ok(Briefing {
            goal: goal.to_string(),
            active_claims,
            relevant,
            recent_decisions,
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
        store
            .progress("s", "a", "did a thing", &["f1".into()])
            .unwrap();
        let recent = store.activity(10).unwrap();
        assert_eq!(recent[0].kind, "progress");
        assert_eq!(recent[1].kind, "claim");
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
                scope: Scope {
                    user_id: None,
                    agent_id: None,
                    project_id: "demo".into(),
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
}
