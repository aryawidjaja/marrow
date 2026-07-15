//! One typed relationship model shared by associative recall and graph visualization.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::embed::cosine;
use crate::index::IndexRow;

const SEMANTIC_MIN_SIM: f32 = 0.55;
const RECALL_SEMANTIC_TOP_K: usize = 5;
const GRAPH_SEMANTIC_TOP_K: usize = 3;
const RECALL_TAG_FANOUT: usize = 12;
const GRAPH_TAG_FANOUT: usize = 7;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeRel {
    Ref,
    Topic,
    Tag,
    Semantic,
    Area,
    Hub,
    Cluster,
    User,
}

impl EdgeRel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ref => "ref",
            Self::Topic => "topic",
            Self::Tag => "tag",
            Self::Semantic => "semantic",
            Self::Area => "area",
            Self::Hub => "hub",
            Self::Cluster => "cluster",
            Self::User => "user",
        }
    }
}

impl fmt::Display for EdgeRel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    Proven,
    Inferred,
    Structural,
    Ambiguous,
}

/// A relationship between two memory or visualization nodes.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub rel: EdgeRel,
    pub trust: Trust,
    pub confidence: f32,
}

impl Edge {
    pub fn new(source: impl Into<String>, target: impl Into<String>, rel: EdgeRel) -> Self {
        let (trust, confidence) = match rel {
            EdgeRel::Ref | EdgeRel::User => (Trust::Proven, 1.0),
            EdgeRel::Topic => (Trust::Inferred, 0.85),
            EdgeRel::Tag => (Trust::Inferred, 0.72),
            EdgeRel::Semantic => (Trust::Inferred, 0.60),
            EdgeRel::Area | EdgeRel::Hub | EdgeRel::Cluster => (Trust::Structural, 1.0),
        };
        Self {
            source: source.into(),
            target: target.into(),
            rel,
            trust,
            confidence,
        }
    }

    pub fn semantic(
        source: impl Into<String>,
        target: impl Into<String>,
        cosine_similarity: f32,
    ) -> Self {
        let mut edge = Self::new(source, target, EdgeRel::Semantic);
        edge.confidence = cosine_similarity.clamp(0.0, 1.0);
        edge
    }

    /// Preserve the established recall strengths. Confidence remains orthogonal trust metadata;
    /// semantic is the one relation whose evidence strength is intrinsically its cosine score.
    pub fn activation_weight(&self) -> f32 {
        match self.rel {
            EdgeRel::Ref | EdgeRel::User => 3.0,
            EdgeRel::Topic => 2.0,
            EdgeRel::Tag => 1.0,
            EdgeRel::Semantic => self.confidence,
            EdgeRel::Area | EdgeRel::Hub | EdgeRel::Cluster => 0.0,
        }
    }

    pub fn recall_label(&self) -> &'static str {
        match self.rel {
            EdgeRel::Ref => "link",
            EdgeRel::Semantic => "meaning",
            _ => self.rel.as_str(),
        }
    }
}

/// Indexed active-memory corpus. It owns relationship extraction once, then exposes projections
/// suited to recall (dense per-node adjacency) and visualization (bounded stars).
pub struct EdgeCorpus<'a> {
    rows: Vec<&'a IndexRow>,
    by_id: HashMap<&'a str, &'a IndexRow>,
    topic_of: HashMap<&'a str, Vec<&'a str>>,
    tag_of: HashMap<&'a str, Vec<&'a str>>,
    topic_to_id: HashMap<&'a str, &'a str>,
    vectors: HashMap<String, Vec<f32>>,
}

impl<'a> EdgeCorpus<'a> {
    pub fn new(rows: &'a [IndexRow], vectors: HashMap<String, Vec<f32>>) -> Self {
        let rows: Vec<&IndexRow> = rows.iter().filter(|row| row.status == "active").collect();
        let mut corpus = Self {
            rows,
            by_id: HashMap::new(),
            topic_of: HashMap::new(),
            tag_of: HashMap::new(),
            topic_to_id: HashMap::new(),
            vectors,
        };
        for row in &corpus.rows {
            corpus.by_id.insert(&row.id, row);
            if !row.topic.is_empty() {
                corpus.topic_of.entry(&row.topic).or_default().push(&row.id);
                corpus.topic_to_id.entry(&row.topic).or_insert(&row.id);
            }
            for tag in tags_of(&row.tags) {
                corpus.tag_of.entry(tag).or_default().push(&row.id);
            }
        }
        corpus
    }

    /// Recall projection: every directly related memory, with semantic confidence kept as cosine.
    pub fn edges_from(&self, id: &str) -> Vec<Edge> {
        let Some(row) = self.by_id.get(id).copied() else {
            return Vec::new();
        };
        let mut edges = self.ref_edges_for(row);
        if let Some(ids) = self.topic_of.get(row.topic.as_str()) {
            edges.extend(
                ids.iter()
                    .filter(|target| **target != id)
                    .map(|target| Edge::new(id, *target, EdgeRel::Topic)),
            );
        }
        for tag in tags_of(&row.tags) {
            if let Some(ids) = self.tag_of.get(tag) {
                if ids.len() <= RECALL_TAG_FANOUT {
                    edges.extend(
                        ids.iter()
                            .filter(|target| **target != id)
                            .map(|target| Edge::new(id, *target, EdgeRel::Tag)),
                    );
                }
            }
        }
        edges.extend(self.semantic_from(id, RECALL_SEMANTIC_TOP_K));
        dedup(edges)
    }

    /// Visualization projection: explicit refs plus bounded topic/tag stars and top-k meaning edges.
    pub fn graph_edges(&self) -> Vec<Edge> {
        let mut edges = Vec::new();
        for row in &self.rows {
            edges.extend(self.ref_edges_for(row));
        }
        for members in self.topic_of.values() {
            star(&mut edges, members, EdgeRel::Topic);
        }
        for members in self.tag_of.values() {
            if members.len() <= GRAPH_TAG_FANOUT {
                star(&mut edges, members, EdgeRel::Tag);
            }
        }
        let existing_pairs: HashSet<(String, String)> = edges
            .iter()
            .map(|edge| ordered_pair(&edge.source, &edge.target))
            .collect();
        let mut semantic = Vec::new();
        for row in &self.rows {
            semantic.extend(self.semantic_from(&row.id, GRAPH_SEMANTIC_TOP_K));
        }
        edges.extend(
            dedup_undirected(semantic)
                .into_iter()
                .filter(|edge| !existing_pairs.contains(&ordered_pair(&edge.source, &edge.target))),
        );
        dedup(edges)
    }

    /// Topic/tag fan-out used by the hub-avoidance threshold.
    pub fn degree(&self, id: &str) -> usize {
        let Some(row) = self.by_id.get(id) else {
            return 0;
        };
        let topic = if row.topic.is_empty() {
            0
        } else {
            self.topic_of.get(row.topic.as_str()).map_or(0, Vec::len)
        };
        let tags = tags_of(&row.tags)
            .filter_map(|tag| self.tag_of.get(tag))
            .filter(|ids| ids.len() <= RECALL_TAG_FANOUT)
            .map(Vec::len)
            .sum::<usize>();
        topic + tags
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|row| row.id.as_str())
    }

    pub fn row(&self, id: &str) -> Option<&IndexRow> {
        self.by_id.get(id).copied()
    }

    pub fn resolve_ref(&self, value: &str) -> Option<&str> {
        self.by_id
            .get_key_value(value)
            .map(|(id, _)| *id)
            .or_else(|| self.topic_to_id.get(value).copied())
    }

    fn ref_edges_for(&self, row: &IndexRow) -> Vec<Edge> {
        wiki_refs(&row.body)
            .into_iter()
            .chain(wiki_refs(&row.topic))
            .filter_map(|value| self.resolve_ref(&value))
            .filter(|target| *target != row.id)
            .map(|target| Edge::new(&row.id, target, EdgeRel::Ref))
            .collect()
    }

    fn semantic_from(&self, id: &str, top_k: usize) -> Vec<Edge> {
        let Some(source) = self.vectors.get(id) else {
            return Vec::new();
        };
        let mut scored: Vec<(&str, f32)> = self
            .vectors
            .iter()
            .filter(|(target, vector)| {
                target.as_str() != id
                    && self.by_id.contains_key(target.as_str())
                    && vector.len() == source.len()
            })
            .map(|(target, vector)| (target.as_str(), cosine(source, vector)))
            .filter(|(_, similarity)| *similarity >= SEMANTIC_MIN_SIM)
            .collect();
        scored.sort_by(|(a_id, a), (b_id, b)| b.total_cmp(a).then_with(|| a_id.cmp(b_id)));
        scored
            .into_iter()
            .take(top_k)
            .map(|(target, similarity)| Edge::semantic(id, target, similarity))
            .collect()
    }
}

fn tags_of(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(',').map(str::trim).filter(|tag| !tag.is_empty())
}

fn wiki_refs(text: &str) -> Vec<String> {
    text.match_indices("[[")
        .filter_map(|(start, _)| {
            let rest = &text[start + 2..];
            rest.find("]]").map(|end| rest[..end].trim().to_string())
        })
        .filter(|value| !value.is_empty() && value.len() <= 48)
        .collect()
}

fn star(edges: &mut Vec<Edge>, members: &[&str], rel: EdgeRel) {
    if let Some((hub, rest)) = members.split_first() {
        edges.extend(rest.iter().map(|target| Edge::new(*hub, *target, rel)));
    }
}

fn dedup(edges: Vec<Edge>) -> Vec<Edge> {
    let mut seen = HashSet::new();
    edges
        .into_iter()
        .filter(|edge| seen.insert((edge.source.clone(), edge.target.clone(), edge.rel)))
        .collect()
}

fn dedup_undirected(edges: Vec<Edge>) -> Vec<Edge> {
    let mut seen = HashSet::new();
    edges
        .into_iter()
        .filter_map(|mut edge| {
            let key = if edge.source <= edge.target {
                (edge.source.clone(), edge.target.clone(), edge.rel)
            } else {
                (edge.target.clone(), edge.source.clone(), edge.rel)
            };
            if !seen.insert(key.clone()) {
                return None;
            }
            edge.source = key.0;
            edge.target = key.1;
            Some(edge)
        })
        .collect()
}

fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, topic: &str, tags: &str, body: &str) -> IndexRow {
        IndexRow {
            id: id.into(),
            kind: "fact".into(),
            status: "active".into(),
            topic: topic.into(),
            area: String::new(),
            project_id: "test".into(),
            written_by: "test".into(),
            model: String::new(),
            confidence: 1.0,
            created_at: String::new(),
            updated_at: String::new(),
            expires_at: String::new(),
            tags: tags.into(),
            path: String::new(),
            body: body.into(),
        }
    }

    #[test]
    fn projections_share_relations_without_forcing_the_same_shape() {
        let rows = vec![
            row("a", "auth", "security", "see [[c]]"),
            row("b", "auth", "security", "second"),
            row("c", "billing", "", "third"),
        ];
        let corpus = EdgeCorpus::new(&rows, HashMap::new());
        let recall = corpus.edges_from("b");
        assert!(recall
            .iter()
            .any(|edge| edge.target == "a" && edge.rel == EdgeRel::Topic));
        let graph = corpus.graph_edges();
        assert!(graph.iter().any(|edge| edge.rel == EdgeRel::Ref));
        assert!(graph.iter().all(|edge| edge.source != edge.target));
    }

    #[test]
    fn semantic_confidence_is_the_actual_similarity() {
        let rows = vec![row("a", "a", "", "a"), row("b", "b", "", "b")];
        let vectors = HashMap::from([("a".into(), vec![1.0, 0.0]), ("b".into(), vec![0.8, 0.6])]);
        let corpus = EdgeCorpus::new(&rows, vectors);
        let edge = corpus
            .edges_from("a")
            .into_iter()
            .find(|edge| edge.rel == EdgeRel::Semantic)
            .unwrap();
        assert!((edge.confidence - 0.8).abs() < 1e-5);
        assert!((edge.activation_weight() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn serde_contract_matches_the_dashboard_and_recall_weights_stay_stable() {
        let reference = Edge::new("a", "b", EdgeRel::Ref);
        let value = serde_json::to_value(&reference).unwrap();
        assert_eq!(value["rel"], "ref");
        assert_eq!(value["trust"], "proven");
        assert_eq!(reference.activation_weight(), 3.0);
        assert_eq!(Edge::new("a", "b", EdgeRel::Topic).activation_weight(), 2.0);
        assert_eq!(Edge::new("a", "b", EdgeRel::Tag).activation_weight(), 1.0);
    }
}
