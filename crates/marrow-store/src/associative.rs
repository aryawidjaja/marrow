//! Associative recall: one fetch returns the direct matches *and* the memories connected to them —
//! by explicit `[[id]]` links, shared topic, shared tag, and related meaning — the way recalling
//! one thing lights up related things in a brain. Neighbours come back terse so the extra context
//! costs little; the agent reads the full text of only the ones it wants.
//!
//! Activation spreads outward for several hops, weakening at each one, so a memory that matches
//! none of your words still surfaces when it sits behind one that does. Memories the agents keep
//! reaching for light up more easily than ones nobody has ever used.

use std::collections::{BTreeSet, HashMap, HashSet};

use marrow_memdocs::{Memory, RefKind};

use crate::edge::{Edge, EdgeCorpus, EdgeRel};
use crate::query::Query;
use crate::store::{Error, Store};

/// A memory connected to the recall's seeds, with why it lit up, how strongly, and how many links
/// away from the nearest direct match it sits.
pub struct Neighbor {
    pub memory: Memory,
    pub via: Vec<String>,
    pub activation: f32,
    pub hops: u8,
}

/// The result of an associative recall: the direct hits plus their connected neighbourhood.
pub struct ConnectedRecall {
    pub seeds: Vec<Memory>,
    pub neighbors: Vec<Neighbor>,
}

/// How far activation travels. Each hop out is worth [`HOP_DECAY`] of the last, so a distant memory
/// has to be strongly connected to beat a weakly connected near one. A few rings is the useful
/// range: past that everything is connected to everything and the ranking stops meaning anything.
const MAX_HOPS: u8 = 3;
const HOP_DECAY: f32 = 0.4;
/// Only the strongest memories found at a hop go on to spread further, so a single hub-like note
/// cannot drag the whole store into the result.
const FRONTIER: usize = 6;
/// Spreading from a seed costs a scan of the corpus, so only the best matches spread. An unbounded
/// search ("every memory mentions auth") would otherwise make the whole recall quadratic.
const SEEDS_SPREAD: usize = 10;
/// A memory has to light up at least this much to be worth spreading from.
const SPREAD_MIN: f32 = 0.35;
/// A node this connected (or above the corpus's 99th-degree percentile) is a hub: activation may
/// REACH it, but spreading it further would drag its whole star in, so hops never transit through
/// it. Seeds are exempt — a hub the agent directly matched is still explored.
const HUB_FLOOR: usize = 50;

/// How much being useful in the past helps a memory surface again. A memory recalled many times
/// ends up worth at most this much more than one never recalled — enough to break ties in favour of
/// what the agents actually use, never enough to bury a strong fresh match.
const USE_BOOST: f32 = 0.6;

/// The Hebbian multiplier for a memory recalled `n` times. Logarithmic, so the tenth recall matters
/// far less than the first, and one much-used memory can't dominate the graph.
fn use_boost(n: u32) -> f32 {
    if n == 0 {
        return 1.0;
    }
    1.0 + USE_BOOST * (1.0 + n as f32).ln() / (1.0 + 50.0f32).ln()
}

/// What a memory has accumulated so far during the spread.
#[derive(Default)]
struct Activation {
    score: f32,
    via: BTreeSet<String>,
    hops: u8,
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

        let rows = self.list()?;
        let corpus = EdgeCorpus::new(
            &rows,
            self.vectors().unwrap_or_default().into_iter().collect(),
        );
        let used = self.recall_counts().unwrap_or_default();

        // Hubs are not transited through during hops (below). Threshold = max(floor, p99 degree),
        // so small stores never trip the cap and large ones cap only their genuine super-hubs.
        let hub_threshold = {
            let mut degs: Vec<usize> = corpus.ids().map(|id| corpus.degree(id)).collect();
            degs.sort_unstable();
            let p99 = degs
                .get((degs.len().saturating_mul(99) / 100).min(degs.len().saturating_sub(1)))
                .copied()
                .unwrap_or(0);
            HUB_FLOOR.max(p99)
        };

        let mut act: HashMap<String, Activation> = HashMap::new();
        // Seeds spread first. Their frontmatter refs are authoritative, so they go in alongside the
        // links parsed from the body.
        for seed in seeds.iter().take(SEEDS_SPREAD) {
            let fm = &seed.frontmatter;
            let mut edges = corpus.edges_from(&fm.id);
            for r in fm.refs.iter().filter(|r| r.kind == RefKind::MemoryId) {
                if let Some(target) = corpus.resolve_ref(&r.value) {
                    edges.push(Edge::new(&fm.id, target, EdgeRel::Ref));
                }
            }
            spread(&mut act, &edges, 1.0, 1, &used);
        }

        // Then hop outward. Each ring is worth less than the last, and only the strongest memories
        // found in a ring go on to spread, so this stays bounded however dense the graph is.
        let mut spread_from: HashSet<String> = seed_ids.clone();
        for hop in 2..=MAX_HOPS {
            let mut next: Vec<(String, f32)> = act
                .iter()
                .filter(|(id, a)| {
                    a.hops == hop - 1 && a.score >= SPREAD_MIN && !spread_from.contains(*id)
                })
                .map(|(id, a)| (id.clone(), a.score))
                .collect();
            next.sort_by(|a, b| b.1.total_cmp(&a.1));
            next.truncate(FRONTIER);
            if next.is_empty() {
                break;
            }
            let decay = HOP_DECAY.powi(hop as i32 - 1);
            for (id, _) in &next {
                spread_from.insert(id.clone());
                // Don't transit through a hub: it can be a neighbour, but spreading its whole
                // topic/tag star would flood recall. (Seeds spread earlier, ungated.)
                if corpus.degree(id) >= hub_threshold {
                    continue;
                }
                let Some(_row) = corpus.row(id) else {
                    continue;
                };
                let edges = corpus.edges_from(id);
                spread(&mut act, &edges, decay, hop, &used);
            }
        }

        for id in &seed_ids {
            act.remove(id);
        }
        let mut ranked: Vec<(String, Activation)> = act.into_iter().collect();
        ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score));
        ranked.truncate(max_neighbors);

        let neighbors = ranked
            .into_iter()
            .filter_map(|(id, a)| {
                self.read(&id).ok().flatten().map(|memory| Neighbor {
                    memory,
                    via: a.via.into_iter().collect(),
                    activation: a.score,
                    hops: a.hops,
                })
            })
            .collect();
        Ok(ConnectedRecall { seeds, neighbors })
    }
}

/// Add one node's outgoing edges into the running activation, damped by how far out we are and
/// lifted by how often each target has proved useful before. `hops` is recorded as the *shortest*
/// path found, so a memory reachable both near and far is reported at its nearest.
fn spread(
    act: &mut HashMap<String, Activation>,
    edges: &[Edge],
    decay: f32,
    hop: u8,
    used: &HashMap<String, u32>,
) {
    for edge in edges {
        let gain = edge.activation_weight()
            * decay
            * use_boost(used.get(&edge.target).copied().unwrap_or(0));
        let e = act.entry(edge.target.clone()).or_default();
        e.score += gain;
        e.via.insert(edge.recall_label().to_string());
        e.hops = if e.hops == 0 { hop } else { e.hops.min(hop) };
    }
}

#[cfg(test)]
mod tests {
    use marrow_memdocs::{Frontmatter, MemoryKind, Provenance, Scope, Status};

    use super::*;

    pub(super) fn fact(topic: &str, body: &str) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Fact,
                status: Status::Active,
                topic: Some(topic.into()),
                area: None,
                scope: Scope {
                    project_id: String::new(),
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
        }
    }

    #[test]
    fn activation_reaches_a_memory_two_links_away() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        // A chain: the query hits A, A links B, B links C. C shares no word with the query, and is
        // not connected to A at all. Only spreading past the first hop can find it.
        let c = store
            .write(&mut fact("rotation", "Signing keys rotate every 90 days."))
            .unwrap();
        let b = store
            .write(&mut fact(
                "signing",
                &format!("Webhooks are signed. See [[{c}]]."),
            ))
            .unwrap();
        store
            .write(&mut fact(
                "sessions",
                &format!("Use JWT for sessions. See [[{b}]]."),
            ))
            .unwrap();
        store
            .write(&mut fact("weather", "The sky is blue today."))
            .unwrap();

        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let r = store.recall_connected("JWT", &q, "test", 8).unwrap();
        let far = r
            .neighbors
            .iter()
            .find(|n| n.memory.frontmatter.id == c)
            .expect("C is two hops out and must still surface");
        assert_eq!(far.hops, 2, "C should be reported at its true distance");

        // The near neighbour still outranks the far one: distance has to cost something.
        let near = r
            .neighbors
            .iter()
            .find(|n| n.memory.frontmatter.id == b)
            .unwrap();
        assert!(
            near.activation > far.activation,
            "one hop ({}) should beat two ({})",
            near.activation,
            far.activation
        );
    }

    #[test]
    fn nothing_connected_means_nothing_lights_up() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .write(&mut fact("sessions", "Use JWT for sessions."))
            .unwrap();
        for i in 0..20 {
            store
                .write(&mut fact(
                    &format!("unrelated-{i}"),
                    "Gardening notes about tulips.",
                ))
                .unwrap();
        }
        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let r = store.recall_connected("JWT", &q, "test", 50).unwrap();
        assert!(
            r.neighbors.is_empty(),
            "nothing is connected, so nothing should light up; got {}",
            r.neighbors.len()
        );
    }

    #[test]
    fn distance_costs_activation_so_a_hub_cannot_flatten_the_ranking() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        // A hub two hops from the query, with a crowd of memories hanging off it. Without decay the
        // whole crowd would rank level with the seed's own direct neighbour.
        let mut crowd = Vec::new();
        for i in 0..12 {
            crowd.push(
                store
                    .write(&mut fact(
                        "hub",
                        &format!("Crowd note {i} about deployment."),
                    ))
                    .unwrap(),
            );
        }
        let hub = store.write(&mut fact("hub", "The hub note.")).unwrap();
        let near = store
            .write(&mut fact(
                "signing",
                &format!("Webhooks are signed. See [[{hub}]]."),
            ))
            .unwrap();
        store
            .write(&mut fact(
                "sessions",
                &format!("Use JWT for sessions. See [[{near}]]."),
            ))
            .unwrap();

        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let r = store.recall_connected("JWT", &q, "test", 40).unwrap();
        let act = |id: &str| {
            r.neighbors
                .iter()
                .find(|n| n.memory.frontmatter.id == id)
                .map(|n| n.activation)
                .unwrap_or(0.0)
        };
        let far = crowd.iter().map(|id| act(id)).fold(0.0f32, f32::max);
        assert!(
            act(&near) > far,
            "the direct neighbour ({}) must outrank everything hanging off the distant hub ({})",
            act(&near),
            far
        );
    }

    #[test]
    fn a_memory_the_agents_keep_using_outranks_one_they_never_touch() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        // Two neighbours connected to the seed in exactly the same way, so the only thing that can
        // separate them is how often each has proved useful.
        let used = store
            .write(&mut fact("billing", "Invoices retry three times."))
            .unwrap();
        let never = store
            .write(&mut fact("billing", "Invoices are PDFs."))
            .unwrap();
        store
            .write(&mut fact("billing", "Use JWT for sessions."))
            .unwrap();

        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let before = store.recall_connected("JWT", &q, "test", 8).unwrap();
        let a = |r: &ConnectedRecall, id: &str| {
            r.neighbors
                .iter()
                .find(|n| n.memory.frontmatter.id == id)
                .map(|n| n.activation)
                .unwrap_or(0.0)
        };
        assert!(
            (a(&before, &used) - a(&before, &never)).abs() < 1e-6,
            "they must start tied, or this test proves nothing"
        );

        for _ in 0..10 {
            store
                .log_retrieval(
                    "agent",
                    "how do invoice retries work",
                    std::slice::from_ref(&used),
                )
                .unwrap();
        }

        let after = store.recall_connected("JWT", &q, "test", 8).unwrap();
        assert!(
            a(&after, &used) > a(&after, &never),
            "the much-used memory should now light up more easily ({} vs {})",
            a(&after, &used),
            a(&after, &never)
        );
    }

    #[test]
    fn recall_counts_rebuild_from_the_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let id = store
            .write(&mut fact("billing", "Invoices retry three times."))
            .unwrap();
        for _ in 0..3 {
            store
                .log_retrieval("agent", "retries", std::slice::from_ref(&id))
                .unwrap();
        }
        // The counts are a derived cache. Losing them must not lose the history.
        store.reindex().unwrap();
        assert_eq!(store.recall_counts().unwrap().get(&id), Some(&3));
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

#[cfg(test)]
mod scale {
    use super::*;
    use crate::store::Store;

    /// Recall is linear in the size of the brain and the spread is bounded, so it degrades gently.
    /// This guards the shape, not the milliseconds: it only trips on a catastrophic regression like
    /// an accidental quadratic or an unbounded number of hops, never on ordinary machine jitter.
    #[test]
    fn recall_stays_fast_on_a_densely_connected_brain() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let mut ids: Vec<String> = Vec::new();
        // The worst case: shared topics, shared tags, and every memory links the one before it, so
        // activation genuinely spreads at every hop instead of dying out immediately.
        for i in 0..1000 {
            let mut m = super::tests::fact(
                &format!("topic-{}", i % 20),
                &format!(
                    "Memory {i} about deployment, auth, billing and infrastructure. See [[{}]].",
                    ids.last().cloned().unwrap_or_default()
                ),
            );
            m.frontmatter.tags = vec![format!("tag-{}", i % 8)];
            ids.push(store.write(&mut m).unwrap());
        }

        let q = Query {
            limit: Some(5),
            ..Default::default()
        };
        let start = std::time::Instant::now();
        let r = store
            .recall_connected("deployment auth", &q, "bench", 8)
            .unwrap();
        let took = start.elapsed();

        assert!(r.neighbors.len() <= 8, "the caller's cap must hold");
        assert!(
            took < std::time::Duration::from_secs(2),
            "recall over 1000 fully-connected memories took {took:?}; something has gone quadratic"
        );
    }
}
