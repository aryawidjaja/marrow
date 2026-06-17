//! Consolidation: the pass that keeps semantic memory coherent.
//!
//! Detection finds memories that need attention — code that drifted (stale anchors),
//! memories past their expiry, and duplicate clusters. Applying it *distills rather than
//! drops*: duplicate clusters are merged into a single memory (its body summarized), expired
//! memories are retired, lineage is preserved via supersede, and every action is recorded in
//! the audit ledger so nothing is lost.

use std::path::Path;

use marrow_memdocs::Status;

use crate::staleness::StaleHit;
use crate::store::{Error, Store};
use crate::{util, Query};

/// Distills several memory bodies into one. The default is deterministic; an LLM-backed
/// implementation can be substituted for richer summaries.
pub trait Summarizer: Send + Sync {
    fn summarize(&self, texts: &[String]) -> Result<String, String>;
}

/// A dependency-free summarizer: keeps the unique, non-empty lines across the inputs in
/// order. Good enough to merge near-identical notes without losing information.
pub struct HeuristicSummarizer;

impl Summarizer for HeuristicSummarizer {
    fn summarize(&self, texts: &[String]) -> Result<String, String> {
        let mut seen = std::collections::HashSet::new();
        let mut lines = Vec::new();
        for text in texts {
            for line in text.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
                    lines.push(trimmed.to_string());
                }
            }
        }
        Ok(lines.join("\n"))
    }
}

/// A set of memories that say the same thing; `keep` absorbs the `merge` ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DupCluster {
    pub keep: String,
    pub merge: Vec<String>,
}

/// What a consolidation pass found (read-only).
#[derive(Debug, Default)]
pub struct ConsolidationReport {
    pub stale: Vec<StaleHit>,
    pub expired: Vec<String>,
    pub duplicates: Vec<DupCluster>,
}

/// What applying consolidation changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConsolidationOutcome {
    pub deprecated: usize,
    pub merged: usize,
}

impl Store {
    /// Detect what needs consolidating, without changing anything.
    pub fn consolidate(&self, repo_root: &Path) -> Result<ConsolidationReport, Error> {
        Ok(ConsolidationReport {
            stale: self.list_stale(repo_root)?,
            expired: self.expired_ids()?,
            duplicates: self.duplicate_clusters()?,
        })
    }

    /// Apply safe consolidation: retire expired memories and merge duplicate clusters into
    /// one (distilling their bodies). Lineage and the audit log preserve what changed.
    pub fn consolidate_apply(&self, _repo_root: &Path) -> Result<ConsolidationOutcome, Error> {
        let mut outcome = ConsolidationOutcome::default();

        for id in self.expired_ids()? {
            if let Some(mut m) = self.read(&id)? {
                m.frontmatter.status = Status::Deprecated;
                self.write(&mut m)?;
                outcome.deprecated += 1;
            }
        }

        for cluster in self.duplicate_clusters()? {
            self.merge_cluster(&cluster)?;
            outcome.merged += cluster.merge.len();
        }

        let summary = format!(
            "consolidated: {} deprecated, {} merged",
            outcome.deprecated, outcome.merged
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

    /// Group active memories whose normalized bodies are identical (>1 per group).
    fn duplicate_clusters(&self) -> Result<Vec<DupCluster>, Error> {
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for row in self.list()? {
            if row.status != "active" {
                continue;
            }
            if let Some(m) = self.read(&row.id)? {
                let key = normalize(&m.body);
                if !key.is_empty() {
                    groups
                        .entry(key)
                        .or_default()
                        .push((m.frontmatter.id, m.frontmatter.updated_at));
                }
            }
        }
        let mut clusters = Vec::new();
        for mut members in groups.into_values() {
            if members.len() < 2 {
                continue;
            }
            // Keep the most recently updated; merge the rest.
            members.sort_by(|a, b| b.1.cmp(&a.1));
            let keep = members[0].0.clone();
            let merge = members[1..].iter().map(|(id, _)| id.clone()).collect();
            clusters.push(DupCluster { keep, merge });
        }
        Ok(clusters)
    }

    /// Merge a duplicate cluster: distill the bodies into `keep`, mark the rest superseded.
    fn merge_cluster(&self, cluster: &DupCluster) -> Result<(), Error> {
        let Some(mut keep) = self.read(&cluster.keep)? else {
            return Ok(());
        };
        let mut bodies = vec![keep.body.clone()];
        for id in &cluster.merge {
            if let Some(m) = self.read(id)? {
                bodies.push(m.body.clone());
            }
        }
        // Retire the merged memories first so a single active memory remains per topic.
        for id in &cluster.merge {
            if let Some(mut m) = self.read(id)? {
                m.frontmatter.status = Status::Superseded;
                self.write(&mut m)?;
            }
        }
        keep.body = self.summarizer.summarize(&bodies).map_err(Error::Db)?;
        for id in &cluster.merge {
            if !keep.frontmatter.supersedes.contains(id) {
                keep.frontmatter.supersedes.push(id.clone());
            }
        }
        self.write(&mut keep)?;
        Ok(())
    }
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
    fn heuristic_summarizer_keeps_unique_lines() {
        let s = HeuristicSummarizer;
        let out = s
            .summarize(&["a\nb".to_string(), "b\nc".to_string()])
            .unwrap();
        assert_eq!(out, "a\nb\nc");
    }
}
