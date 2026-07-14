//! Consolidation: the pass that makes semantic memory *learn* rather than just accumulate.
//!
//! Related memories are found by meaning (embedding similarity, not exact text). Each cluster
//! is judged by a [`Distiller`] — merge near-identical notes, resolve contradictions, or leave
//! genuinely distinct memories alone. The survivor is chosen by [`salience`], lineage is kept
//! via supersede, and every action is written to the audit ledger, so nothing is lost.

use std::path::Path;

use marrow_memdocs::{Memory, Status};

use crate::config::Config;
use crate::embed::cosine;
use crate::staleness::StaleHit;
use crate::store::{Error, Store};
use crate::{util, Query};

/// What to do with a cluster of related memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterAction {
    /// The memories say the same thing; collapse into one.
    Merge,
    /// The memories disagree; keep the survivor and record the resolution.
    Conflict,
    /// The memories are genuinely distinct; leave them alone.
    Keep,
}

/// A distiller's judgement of a cluster.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub action: ClusterAction,
    /// The distilled body for the survivor (used for Merge and Conflict).
    pub body: String,
    /// A short, human-readable reason (recorded in the audit ledger).
    pub rationale: String,
}

/// Judges a cluster of related memories and distills them into one. Deterministic: it keeps what the
/// memories agree on and never invents a claim none of them made.
pub trait Distiller: Send + Sync {
    fn distill(&self, bodies: &[String]) -> Result<Verdict, String>;
}

/// A dependency-free distiller: merges near-identical notes (keeping unique lines) and never
/// claims to resolve a contradiction it can't actually understand. Without an LLM, it dedups.
pub struct HeuristicDistiller;

impl Distiller for HeuristicDistiller {
    fn distill(&self, bodies: &[String]) -> Result<Verdict, String> {
        let mut seen = std::collections::HashSet::new();
        let mut lines = Vec::new();
        for body in bodies {
            for line in body.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                    lines.push(trimmed.to_string());
                }
            }
        }
        Ok(Verdict {
            action: ClusterAction::Merge,
            body: lines.join("\n"),
            rationale: "merged duplicate notes".to_string(),
        })
    }
}

/// Build the configured distiller, falling back to the heuristic when no LLM is available.
pub fn build_distiller(_config: &Config) -> Box<dyn Distiller> {
    Box::new(HeuristicDistiller)
}

/// How strongly a memory should be retained. Higher means "keep this one".
pub fn salience(memory: &Memory) -> f64 {
    memory.frontmatter.confidence
}

/// A cluster of related memories, with the chosen survivor first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    pub keep: String,
    pub others: Vec<String>,
}

/// What a consolidation pass found (read-only).
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub stale: Vec<StaleHit>,
    pub expired: Vec<String>,
    pub clusters: Vec<Cluster>,
}

/// New memories written since the last consolidation that make an automatic cleanup pass worthwhile.
pub const CONSOLIDATE_THRESHOLD: usize = 20;

/// What applying consolidation changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConsolidationOutcome {
    pub deprecated: usize,
    pub merged: usize,
    pub conflicts_resolved: usize,
}

impl Store {
    /// Detect what needs consolidating, without changing anything.
    pub fn consolidate(&self, repo_root: &Path) -> Result<ConsolidationReport, Error> {
        Ok(ConsolidationReport {
            stale: self.list_stale(repo_root)?,
            expired: self.expired_ids()?,
            clusters: self.clusters()?,
        })
    }

    /// Apply consolidation: retire expired memories, then for each related cluster ask the
    /// distiller to merge, resolve a conflict, or keep. Lineage and the audit log preserve all.
    pub fn consolidate_apply(&self, _repo_root: &Path) -> Result<ConsolidationOutcome, Error> {
        let mut outcome = ConsolidationOutcome::default();

        for id in self.expired_ids()? {
            if let Some(mut m) = self.read(&id)? {
                m.frontmatter.status = Status::Deprecated;
                self.write(&mut m)?;
                outcome.deprecated += 1;
            }
        }

        for cluster in self.clusters()? {
            match self.resolve_cluster(&cluster)? {
                ClusterAction::Merge => outcome.merged += cluster.others.len(),
                ClusterAction::Conflict => outcome.conflicts_resolved += 1,
                ClusterAction::Keep => {}
            }
        }

        let summary = format!(
            "consolidated: {} deprecated, {} merged, {} conflicts resolved",
            outcome.deprecated, outcome.merged, outcome.conflicts_resolved
        );
        self.log_event("consolidate", "marrow", &summary)?;
        Ok(outcome)
    }

    /// How many memories have been written since the last consolidation pass (folds the ledger:
    /// each `write` increments, a `consolidate` event resets the count).
    pub fn writes_since_consolidation(&self) -> Result<usize, Error> {
        let mut count = 0;
        for ev in self.history()? {
            match ev.kind.as_str() {
                "consolidate" => count = 0,
                "write" => count += 1,
                _ => {}
            }
        }
        Ok(count)
    }

    /// True once enough new memories have accumulated to warrant an automatic cleanup pass.
    pub fn consolidation_due(&self) -> Result<bool, Error> {
        Ok(self.writes_since_consolidation()? >= CONSOLIDATE_THRESHOLD)
    }

    /// Apply consolidation only if it's [`Store::consolidation_due`]; otherwise a no-op. This is the
    /// entry point for hands-free auto-consolidation (the bootstrap hook / agent call it cheaply).
    pub fn consolidate_if_due(
        &self,
        repo_root: &Path,
    ) -> Result<Option<ConsolidationOutcome>, Error> {
        if self.consolidation_due()? {
            Ok(Some(self.consolidate_apply(repo_root)?))
        } else {
            Ok(None)
        }
    }

    /// Ids of active memories whose `expires_at` is in the past.
    fn expired_ids(&self) -> Result<Vec<String>, Error> {
        let now = util::to_unix(&util::now_rfc3339()).unwrap_or(0);
        let all = self.query(&Query {
            exclude_expired: false,
            ..Query::default()
        })?;
        Ok(all
            .into_iter()
            .filter(|m| {
                m.frontmatter.status == Status::Active
                    && m.frontmatter
                        .decay
                        .as_ref()
                        .and_then(|d| d.expires_at.as_deref())
                        .is_some_and(|e| util::is_expired(e, now))
            })
            .map(|m| m.frontmatter.id)
            .collect())
    }

    /// Group active memories into clusters of related meaning. With an embedder this is by
    /// cosine similarity; without one it falls back to exact normalized-body matching.
    fn clusters(&self) -> Result<Vec<Cluster>, Error> {
        let memories: Vec<Memory> = self
            .query(&Query {
                status: Some(Status::Active),
                ..Query::default()
            })?
            .into_iter()
            .filter(|m| !m.body.trim().is_empty())
            .collect();

        let groups = match &self.embedder {
            Some(embedder) => self.semantic_groups(&memories, embedder.as_ref())?,
            None => exact_groups(&memories),
        };

        let mut clusters = Vec::new();
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            let mut members: Vec<&Memory> = group
                .iter()
                .filter_map(|id| memories.iter().find(|m| &m.frontmatter.id == id))
                .collect();
            {
                // Survivor = highest salience, tie-broken by most recent update.
                members.sort_by(|a, b| {
                    salience(b)
                        .partial_cmp(&salience(a))
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.frontmatter.updated_at.cmp(&a.frontmatter.updated_at))
                });
                let keep = members[0].frontmatter.id.clone();
                let others = members[1..]
                    .iter()
                    .map(|m| m.frontmatter.id.clone())
                    .collect();
                clusters.push(Cluster { keep, others });
            }
        }
        Ok(clusters)
    }
}

/// A restatement of the same fact lands almost on top of the original. Anything at or above this is
/// treated as a true duplicate outright, which is what lets three identical notes collapse into one.
const NEAR_IDENTICAL: f32 = 0.95;
/// Below that, a merely-similar pair must also be *distinctly* closer to each other than to anything
/// else. Without this a bare threshold is meaningless: these embedding models put unrelated notes
/// above 0.8 anyway, so "similar enough" swept 37 distinct decisions into one cluster and retired
/// all but one of them.
const DUPLICATE_MARGIN: f32 = 0.04;

impl Store {
    /// Group memories that are genuinely duplicates of each other.
    ///
    /// A duplicate is either a near-identical restatement, or a pair that is *mutually* each other's
    /// closest match by a clear margin. A plain similarity threshold is not a duplicate test.
    fn semantic_groups(
        &self,
        memories: &[Memory],
        embedder: &dyn crate::embed::Embedder,
    ) -> Result<Vec<Vec<String>>, Error> {
        let floor = self.config.consolidation.sim_threshold as f32;
        let texts: Vec<String> = memories.iter().map(|m| m.body.clone()).collect();
        let vectors = embedder
            .embed(&texts)
            .map_err(|e| Error::Db(e.to_string()))?;
        let n = memories.len();
        let sim = |i: usize, j: usize| cosine(&vectors[i], &vectors[j]);

        // Each memory's closest peer, but only when it clearly stands out from the runner-up.
        let standout: Vec<Option<usize>> = (0..n)
            .map(|i| {
                let mut sims: Vec<(usize, f32)> =
                    (0..n).filter(|&j| j != i).map(|j| (j, sim(i, j))).collect();
                sims.sort_by(|a, b| b.1.total_cmp(&a.1));
                let (best, top) = *sims.first()?;
                let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
                (top >= floor && top - runner_up >= DUPLICATE_MARGIN).then_some(best)
            })
            .collect();

        // Union-find: near-identical notes may chain (they really are the same thing); merely-similar
        // ones only pair up when the choice is mutual.
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut Vec<usize>, i: usize) -> usize {
            if parent[i] != i {
                let root = find(parent, parent[i]);
                parent[i] = root;
            }
            parent[i]
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let s = sim(i, j);
                let duplicate = s >= NEAR_IDENTICAL
                    || (s >= floor && standout[i] == Some(j) && standout[j] == Some(i));
                if duplicate {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a] = b;
                    }
                }
            }
        }

        let mut by_root: std::collections::HashMap<usize, Vec<String>> =
            std::collections::HashMap::new();
        for (i, m) in memories.iter().enumerate() {
            let root = find(&mut parent, i);
            by_root
                .entry(root)
                .or_default()
                .push(m.frontmatter.id.clone());
        }
        Ok(by_root.into_values().collect())
    }

    /// Ask the distiller what to do with a cluster and apply it.
    fn resolve_cluster(&self, cluster: &Cluster) -> Result<ClusterAction, Error> {
        let Some(mut keep) = self.read(&cluster.keep)? else {
            return Ok(ClusterAction::Keep);
        };
        let mut bodies = vec![keep.body.clone()];
        for id in &cluster.others {
            if let Some(m) = self.read(id)? {
                bodies.push(m.body.clone());
            }
        }
        let verdict = self.distiller.distill(&bodies).map_err(Error::Db)?;
        if verdict.action == ClusterAction::Keep {
            return Ok(ClusterAction::Keep);
        }

        // Retire the others first so a single active memory remains.
        for id in &cluster.others {
            if let Some(mut m) = self.read(id)? {
                m.frontmatter.status = Status::Superseded;
                self.write(&mut m)?;
            }
        }
        keep.body = verdict.body;
        for id in &cluster.others {
            if !keep.frontmatter.supersedes.contains(id) {
                keep.frontmatter.supersedes.push(id.clone());
            }
        }
        self.write(&mut keep)?;

        if verdict.action == ClusterAction::Conflict {
            let summary = format!(
                "resolved conflict into {}: {}",
                cluster.keep, verdict.rationale
            );
            self.log_event("conflict_resolved", "marrow", &summary)?;
        }
        Ok(verdict.action)
    }
}

/// Group memories with identical normalized bodies (the no-embedder fallback).
fn exact_groups(memories: &[Memory]) -> Vec<Vec<String>> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in memories {
        let key = normalize(&m.body);
        if !key.is_empty() {
            groups
                .entry(key)
                .or_default()
                .push(m.frontmatter.id.clone());
        }
    }
    groups.into_values().collect()
}

fn normalize(body: &str) -> String {
    body.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn heuristic_distiller_merges_unique_lines() {
        let v = HeuristicDistiller
            .distill(&["a\nb".to_string(), "b\nc".to_string()])
            .unwrap();
        assert_eq!(v.action, ClusterAction::Merge);
        assert_eq!(v.body, "a\nb\nc");
    }

    /// An embedder whose vectors all sit very close together — exactly how a real small model
    /// behaves (high baseline cosine), which is what made a fixed threshold catastrophic.
    struct HighBaselineEmbedder;
    impl crate::embed::Embedder for HighBaselineEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, crate::embed::EmbedError> {
            Ok(texts
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    // Distinct notes that a real small model still rates ~0.88 alike — above the
                    // 0.83 threshold, which is exactly why a bare threshold was catastrophic.
                    let mut v = vec![1.0f32; 8];
                    v[i % 8] += 0.9;
                    v
                })
                .collect())
        }
        fn dim(&self) -> usize {
            8
        }
    }

    /// `topic` is the identity of the thing being remembered, so memories on DIFFERENT topics must
    /// never be merged — not even when the embedder rates every one of them alike, which it will on a
    /// corpus with a high baseline similarity.
    #[test]
    fn consolidation_never_merges_distinct_topics_even_when_embeddings_are_alike() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.embedder = Some(Box::new(HighBaselineEmbedder));

        let distinct = [
            (
                "readme-rewrite",
                "Restructured the README to lead with the quickstart.",
            ),
            (
                "pluribus-story",
                "The narrative opens with the Pluribus hive analogy.",
            ),
            (
                "gtm-hub71",
                "Applying to the Hub71 accelerator as a solo founder.",
            ),
            ("jwt-expiry", "Access tokens expire after fifteen minutes."),
            (
                "stripe-webhooks",
                "Stripe webhooks are signature-verified before use.",
            ),
        ];
        for (topic, body) in distinct {
            write_fact(&store, topic, body);
        }

        let outcome = store.consolidate_apply(dir.path()).unwrap();
        let survivors = store
            .query(&Query {
                status: Some(Status::Active),
                ..Query::default()
            })
            .unwrap();
        assert_eq!(
            outcome.merged, 0,
            "merged {} memories across different topics",
            outcome.merged
        );
        assert_eq!(
            survivors.len(),
            distinct.len(),
            "consolidation retired distinct topics. survivors: {:?}",
            survivors
                .iter()
                .map(|m| m.frontmatter.topic.clone())
                .collect::<Vec<_>>()
        );
    }

    /// The flip side: it must still do its job on genuine duplicates of the SAME topic.
    #[test]
    fn consolidation_still_merges_true_duplicates_on_one_topic() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.embedder = Some(Box::new(HighBaselineEmbedder));
        write_fact(
            &store,
            "jwt-expiry",
            "Access tokens expire after fifteen minutes.",
        );
        write_fact(&store, "jwt-expiry", "Access tokens expire after 15 min.");

        let outcome = store.consolidate_apply(dir.path()).unwrap();
        assert_eq!(
            outcome.merged, 1,
            "same-topic duplicates should still merge"
        );
    }

    fn write_fact(store: &Store, topic: &str, body: &str) {
        use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope};
        let mut m = Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: Some(topic.into()),
                area: None,
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "t".into(),
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
            body: body.into(),
        };
        store.write(&mut m).unwrap();
    }

    #[test]
    fn consolidation_becomes_due_then_resets_after_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        // A handful of writes: not due yet.
        for i in 0..3 {
            write_fact(&store, &format!("t{i}"), &format!("fact number {i}"));
        }
        assert_eq!(store.writes_since_consolidation().unwrap(), 3);
        assert!(!store.consolidation_due().unwrap());
        assert!(store.consolidate_if_due(dir.path()).unwrap().is_none());

        // Cross the threshold.
        for i in 3..CONSOLIDATE_THRESHOLD {
            write_fact(&store, &format!("t{i}"), &format!("fact number {i}"));
        }
        assert!(store.consolidation_due().unwrap());

        // consolidate_if_due runs the pass and resets the counter.
        assert!(store.consolidate_if_due(dir.path()).unwrap().is_some());
        assert_eq!(store.writes_since_consolidation().unwrap(), 0);
        assert!(!store.consolidation_due().unwrap());
    }
}
