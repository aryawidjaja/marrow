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

/// Judges a cluster of related memories and distills them. The default is deterministic; an
/// LLM-backed distiller (feature `distill-http`, pointed at a local/sovereign model) resolves
/// genuine contradictions.
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
pub fn build_distiller(config: &Config) -> Box<dyn Distiller> {
    match config.consolidation.distiller.as_str() {
        #[cfg(feature = "distill-http")]
        "http" => Box::new(crate::distill_http::HttpDistiller::from_config(
            &config.consolidation,
        )),
        _ => Box::new(HeuristicDistiller),
    }
}

/// How strongly a memory should be retained: decayed confidence, tie-broken by recency.
/// Higher means "keep this one".
pub fn salience(memory: &Memory, now_unix: i64) -> f64 {
    let fm = &memory.frontmatter;
    match fm.decay.as_ref().and_then(|d| d.half_life.as_deref()) {
        Some(hl) => util::decayed_confidence(fm.confidence, &fm.created_at, hl, now_unix),
        None => fm.confidence,
    }
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
        let now = util::to_unix(&util::now_rfc3339()).unwrap_or(0);
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
            // Survivor = highest salience, tie-broken by most recent update.
            members.sort_by(|a, b| {
                salience(b, now)
                    .partial_cmp(&salience(a, now))
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
        Ok(clusters)
    }

    /// Cluster memories whose embeddings are within the configured similarity threshold.
    fn semantic_groups(
        &self,
        memories: &[Memory],
        embedder: &dyn crate::embed::Embedder,
    ) -> Result<Vec<Vec<String>>, Error> {
        let threshold = self.config.consolidation.sim_threshold;
        let texts: Vec<String> = memories.iter().map(|m| m.body.clone()).collect();
        let vectors = embedder
            .embed(&texts)
            .map_err(|e| Error::Db(e.to_string()))?;

        let n = memories.len();
        let mut assigned = vec![false; n];
        let mut groups = Vec::new();
        for i in 0..n {
            if assigned[i] {
                continue;
            }
            let mut group = vec![memories[i].frontmatter.id.clone()];
            assigned[i] = true;
            for j in (i + 1)..n {
                if !assigned[j] && cosine(&vectors[i], &vectors[j]) >= threshold as f32 {
                    group.push(memories[j].frontmatter.id.clone());
                    assigned[j] = true;
                }
            }
            groups.push(group);
        }
        Ok(groups)
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
}
