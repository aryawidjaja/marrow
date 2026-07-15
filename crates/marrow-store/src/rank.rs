//! Deterministic, embedding-free relevance re-scoring of keyword candidates.
//!
//! Candidates arrive already ordered by full-text bm25 (see [`crate::index::search`]). This module
//! layers a topic-tier boost on top and stably reorders: a strong `topic` match rises above a
//! merely body-relevant one, while everything with no topic signal keeps its incoming bm25 order.

use std::collections::{HashMap, HashSet};

use crate::index::IndexRow;

const EXACT: f64 = 1000.0;
const PREFIX: f64 = 100.0;
const SUBSTRING: f64 = 1.0;
const AREA_TAG: f64 = 0.5;

/// Split a query into lowercased, deduped, order-preserving terms (alphanumeric runs).
fn terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
    {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

/// IDF per term over the candidate set: `ln(1 + N/(1+df))`, df = #candidates whose topic contains t.
fn idf(rows: &[IndexRow], terms: &[String]) -> HashMap<String, f64> {
    let n = rows.len().max(1) as f64;
    let mut df: HashMap<String, usize> = terms.iter().map(|t| (t.clone(), 0)).collect();
    for r in rows {
        let topic = r.topic.to_lowercase();
        for t in terms {
            if topic.contains(t.as_str()) {
                *df.get_mut(t).unwrap() += 1;
            }
        }
    }
    terms
        .iter()
        .map(|t| (t.clone(), (1.0 + n / (1.0 + df[t] as f64)).ln()))
        .collect()
}

/// Topic-tier boost for one candidate. `terms` are the query terms; `idf` their weights.
fn topic_boost(row: &IndexRow, terms: &[String], idf: &HashMap<String, f64>) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let topic = row.topic.to_lowercase();
    let area = row.area.to_lowercase();
    let tags = row.tags.to_lowercase();
    let joined = terms.join(" ");
    let idf_max = terms
        .iter()
        .map(|t| idf.get(t).copied().unwrap_or(1.0))
        .fold(1.0_f64, f64::max);

    let mut score = 0.0;
    // Whole-query tier: the full query equals/prefixes the topic.
    if topic == joined {
        score += EXACT * 10.0 * idf_max;
    } else if topic.starts_with(&joined) {
        score += PREFIX * 10.0 * idf_max;
    }

    let mut matched = 0usize;
    let mut tiered = 0.0;
    for t in terms {
        let w = idf.get(t).copied().unwrap_or(1.0);
        // Strongest tier per term: exact > prefix > substring (take one only).
        if topic == *t {
            tiered += EXACT * w;
            matched += 1;
        } else if topic.starts_with(t.as_str()) {
            tiered += PREFIX * w;
            matched += 1;
        } else if topic.contains(t.as_str()) {
            score += SUBSTRING * w;
            matched += 1;
        }
        // area/tag hits score but do NOT count toward coverage.
        if area.contains(t.as_str()) || tags.contains(t.as_str()) {
            score += AREA_TAG * w;
        }
    }
    let coverage = (matched as f64 / terms.len() as f64).powi(2);
    score + tiered * coverage
}

/// Stably reorder bm25-ordered candidates by topic boost (desc), tie-break shorter topic.
/// Candidates with equal boost keep their incoming (bm25) order.
pub fn rerank(rows: Vec<IndexRow>, query: &str) -> Vec<IndexRow> {
    let terms = terms(query);
    let idf = idf(&rows, &terms);
    // Per-row (topic_boost, topic_char_len) key.
    let keys: Vec<(f64, usize)> = rows
        .iter()
        .map(|r| (topic_boost(r, &terms, &idf), r.topic.chars().count()))
        .collect();
    // Stable sort of indices by (-boost, topic_len); equal keys retain bm25 order.
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    idx.sort_by(|&a, &b| {
        let by_boost = keys[b]
            .0
            .partial_cmp(&keys[a].0)
            .unwrap_or(std::cmp::Ordering::Equal);
        // Shorter-topic tie-break only among genuinely-tied POSITIVE boosts; zero-boost
        // candidates keep their incoming bm25 order (stable sort → no length reshuffle).
        if by_boost == std::cmp::Ordering::Equal && keys[a].0 > 0.0 {
            keys[a].1.cmp(&keys[b].1)
        } else {
            by_boost
        }
    });
    let mut taken: Vec<Option<IndexRow>> = rows.into_iter().map(Some).collect();
    idx.into_iter().map(|i| taken[i].take().unwrap()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, topic: &str, area: &str, tags: &str) -> IndexRow {
        IndexRow {
            id: id.into(),
            kind: "fact".into(),
            status: "active".into(),
            topic: topic.into(),
            area: area.into(),
            project_id: String::new(),
            written_by: String::new(),
            model: String::new(),
            confidence: 1.0,
            created_at: String::new(),
            updated_at: String::new(),
            expires_at: String::new(),
            tags: tags.into(),
            path: String::new(),
            body: String::new(),
        }
    }

    #[test]
    fn exact_beats_prefix_beats_substring() {
        // bm25 order (input) is deliberately worst-first to prove reranking overrides it.
        let rows = vec![
            row("sub", "the release notes", "", ""), // substring "release"
            row("pre", "release-process", "", ""),   // prefix "release"
            row("exa", "release", "", ""),           // exact "release"
        ];
        let out = rerank(rows, "release");
        assert_eq!(out[0].id, "exa");
        assert_eq!(out[1].id, "pre");
        assert_eq!(out[2].id, "sub");
    }

    #[test]
    fn coverage_damps_lone_common_word_exact() {
        // "auth" exactly matches a short topic but covers 1/2 terms; the other node
        // prefix+substring-matches both terms and should win via coverage².
        let rows = vec![
            row("lone", "auth", "", ""),
            row("both", "auth token refresh", "", ""),
        ];
        let out = rerank(rows, "auth token");
        assert_eq!(out[0].id, "both");
    }

    #[test]
    fn shorter_topic_breaks_ties() {
        let rows = vec![
            row("long", "billing invoice retry", "", ""),
            row("short", "billing", "", ""),
        ];
        // both prefix-match sole term "billing" (coverage 1); shorter topic wins.
        let out = rerank(rows, "billing");
        assert_eq!(out[0].id, "short");
    }

    #[test]
    fn zero_boost_keeps_bm25_order() {
        let rows = vec![row("first", "alpha", "", ""), row("second", "beta", "", "")];
        let out = rerank(rows, "unrelated");
        assert_eq!(out[0].id, "first");
        assert_eq!(out[1].id, "second");
    }

    #[test]
    fn area_tag_hit_scores_without_counting_coverage() {
        // "infra" appears only in area; the topic match is the single term "deploy".
        let with_area = row("a", "deploy", "infra", "");
        let no_area = row("b", "deploy", "", "");
        let t = terms("deploy infra");
        let rows = vec![with_area.clone(), no_area.clone()];
        let idfm = idf(&rows, &t);
        assert!(topic_boost(&with_area, &t, &idfm) > topic_boost(&no_area, &t, &idfm));
    }
}
