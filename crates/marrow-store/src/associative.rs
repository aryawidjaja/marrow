//! Associative recall: one fetch returns the direct matches *and* the memories connected to them —
//! by explicit `[[id]]` links, shared topic, shared tag, and related meaning — the way recalling
//! one thing lights up related things in a brain. Neighbours come back terse so the extra context
//! costs little; the agent reads the full text of only the ones it wants.

use std::collections::{BTreeSet, HashMap, HashSet};

use marrow_memdocs::{Memory, RefKind};

use crate::embed::cosine;
use crate::query::Query;
use crate::store::{Error, Store};

/// A memory connected to the recall's seeds, with why it lit up and how strongly.
pub struct Neighbor {
    pub memory: Memory,
    pub via: Vec<String>,
    pub activation: f32,
}

/// The result of an associative recall: the direct hits plus their connected neighbourhood.
pub struct ConnectedRecall {
    pub seeds: Vec<Memory>,
    pub neighbors: Vec<Neighbor>,
}

// How much each kind of connection contributes to a neighbour's activation. Explicit agent-authored
// links count most; a shared generic-ish tag least. Meaning is weighted by its actual cosine.
const W_LINK: f32 = 3.0;
const W_TOPIC: f32 = 2.0;
const W_TAG: f32 = 1.0;
const SEM_MIN: f32 = 0.55;
const SEM_TOPK: usize = 5;
/// A tag shared by more than this many memories is a convention, not a link — skip it.
const TAG_FANOUT: usize = 12;

fn wiki_refs(text: &str) -> Vec<String> {
    text.match_indices("[[")
        .filter_map(|(i, _)| {
            text[i + 2..]
                .find("]]")
                .map(|j| text[i + 2..i + 2 + j].trim().to_string())
        })
        .filter(|s| !s.is_empty() && s.len() <= 48)
        .collect()
}

impl Store {
    /// Recall `text`, then expand to the neighbourhood connected to the matches. `max_neighbors`
    /// bounds the extra memories returned. Records the retrieval like [`Store::recall`].
    pub fn recall_connected(
        &self,
        text: &str,
        q: &Query,
        actor: &str,
        max_neighbors: usize,
    ) -> Result<ConnectedRecall, Error> {
        let seeds = self.search(text, q)?;
        let seed_ids: HashSet<String> = seeds.iter().map(|m| m.frontmatter.id.clone()).collect();
        self.log_retrieval(actor, text, &seed_ids.iter().cloned().collect::<Vec<_>>())?;
        if seeds.is_empty() || max_neighbors == 0 {
            return Ok(ConnectedRecall {
                seeds,
                neighbors: vec![],
            });
        }

        // Index the active corpus once: topic → ids, tag → ids, and the set of valid ids/topics.
        let rows = self.list()?;
        let mut topic_of: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut tag_of: HashMap<String, Vec<&str>> = HashMap::new();
        let mut topic_to_id: HashMap<&str, &str> = HashMap::new();
        let mut all_ids: HashSet<&str> = HashSet::new();
        for r in &rows {
            if r.status != "active" {
                continue;
            }
            all_ids.insert(&r.id);
            if !r.topic.is_empty() {
                topic_of.entry(&r.topic).or_default().push(&r.id);
                topic_to_id.entry(&r.topic).or_insert(&r.id);
            }
            for t in r.tags.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                tag_of.entry(t.to_string()).or_default().push(&r.id);
            }
        }
        let vecs: HashMap<String, Vec<f32>> =
            self.vectors().unwrap_or_default().into_iter().collect();

        let mut act: HashMap<String, (f32, BTreeSet<String>)> = HashMap::new();
        let mut bump = |id: &str, w: f32, via: String| {
            let e = act.entry(id.to_string()).or_insert((0.0, BTreeSet::new()));
            e.0 += w;
            e.1.insert(via);
        };

        for seed in &seeds {
            let fm = &seed.frontmatter;
            // Explicit links: [[id]]/[[topic]] in the text, plus any MemoryId refs in frontmatter.
            let mut refs = wiki_refs(&seed.body);
            refs.extend(wiki_refs(fm.topic.as_deref().unwrap_or("")));
            refs.extend(
                fm.refs
                    .iter()
                    .filter(|r| r.kind == RefKind::MemoryId)
                    .map(|r| r.value.clone()),
            );
            for rf in refs {
                let target = if all_ids.contains(rf.as_str()) {
                    Some(rf.clone())
                } else {
                    topic_to_id.get(rf.as_str()).map(|s| s.to_string())
                };
                if let Some(t) = target {
                    bump(&t, W_LINK, "link".into());
                }
            }
            if let Some(topic) = &fm.topic {
                for id in topic_of.get(topic.as_str()).into_iter().flatten() {
                    bump(id, W_TOPIC, "topic".into());
                }
            }
            for tag in &fm.tags {
                if let Some(ids) = tag_of.get(tag) {
                    if ids.len() <= TAG_FANOUT {
                        for id in ids {
                            bump(id, W_TAG, format!("tag:{tag}"));
                        }
                    }
                }
            }
            if let Some(sv) = vecs.get(&fm.id) {
                let mut sims: Vec<(&str, f32)> = vecs
                    .iter()
                    .filter(|(id, v)| id.as_str() != fm.id && v.len() == sv.len())
                    .map(|(id, v)| (id.as_str(), cosine(sv, v)))
                    .filter(|(_, s)| *s >= SEM_MIN)
                    .collect();
                sims.sort_by(|a, b| b.1.total_cmp(&a.1));
                for (id, s) in sims.into_iter().take(SEM_TOPK) {
                    bump(id, s, "meaning".into());
                }
            }
        }

        for id in &seed_ids {
            act.remove(id);
        }
        let mut ranked: Vec<(String, (f32, BTreeSet<String>))> = act.into_iter().collect();
        ranked.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        ranked.truncate(max_neighbors);

        let neighbors = ranked
            .into_iter()
            .filter_map(|(id, (activation, via))| {
                self.read(&id).ok().flatten().map(|memory| Neighbor {
                    memory,
                    via: via.into_iter().collect(),
                    activation,
                })
            })
            .collect();
        Ok(ConnectedRecall { seeds, neighbors })
    }
}

#[cfg(test)]
mod tests {
    use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope, Status};

    use super::*;

    fn fact(topic: &str, body: &str) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: Some(topic.into()),
                area: None,
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
                    written_by: "t".into(),
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

    #[test]
    fn recall_pulls_in_linked_and_same_topic_neighbours() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let b = store
            .write(&mut fact("billing", "Stripe webhooks are signed."))
            .unwrap();
        // A matches "JWT", shares topic "billing" with B, and explicitly links B.
        store
            .write(&mut fact(
                "billing",
                &format!("Use JWT for sessions. See [[{b}]]."),
            ))
            .unwrap();
        let c = store
            .write(&mut fact("weather", "The sky is blue today."))
            .unwrap();

        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let r = store.recall_connected("JWT", &q, "test", 8).unwrap();
        let nids: Vec<&str> = r
            .neighbors
            .iter()
            .map(|n| n.memory.frontmatter.id.as_str())
            .collect();
        assert!(
            nids.contains(&b.as_str()),
            "B should light up via link + shared topic"
        );
        assert!(!nids.contains(&c.as_str()), "unrelated C should not");
        let bn = r
            .neighbors
            .iter()
            .find(|n| n.memory.frontmatter.id == b)
            .unwrap();
        assert!(bn.via.iter().any(|v| v == "link"), "via: {:?}", bn.via);
    }
}
