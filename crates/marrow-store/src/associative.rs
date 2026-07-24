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
use crate::index::IndexRow;
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
pub(crate) const MAX_HOPS: u8 = 3;
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

        // Recall no longer loads the whole project. It walks the bounded candidate set — the query's
        // keyword and vector lanes plus the edge-neighbourhood reachable from them — and spreads over
        // that. The neighbourhood carries the same edges the full scan would for every node the
        // spread can actually reach, so the result matches a whole-store build without the whole-store
        // cost. See [`Store::bounded_corpus`].
        let corpus = self.bounded_corpus(&seeds, text, crate::store::RECALL_CANDIDATES)?;
        let neighbors = self.spread_recall(
            &seeds,
            &seed_ids,
            &corpus.rows,
            corpus.vectors,
            corpus.hub_tags,
            max_neighbors,
        )?;
        Ok(ConnectedRecall { seeds, neighbors })
    }

    /// Spread activation from `seeds` across the graph induced by `rows`+`vectors` and return the
    /// lit-up neighbourhood, best first, capped at `max_neighbors`. This is the recall's ranking
    /// core: the caller decides *which* rows the graph is built from (the bounded candidate set in
    /// production, the whole store in the parity oracle) — the scoring here is identical either way.
    /// `hub_tags` are global hub tags to suppress; empty for the full-store oracle.
    fn spread_recall(
        &self,
        seeds: &[Memory],
        seed_ids: &HashSet<String>,
        rows: &[IndexRow],
        vectors: HashMap<String, Vec<f32>>,
        hub_tags: HashSet<String>,
        max_neighbors: usize,
    ) -> Result<Vec<Neighbor>, Error> {
        let mut corpus = EdgeCorpus::new(rows, vectors);
        corpus.suppress_hub_tags(hub_tags);
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

        for id in seed_ids {
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
        Ok(neighbors)
    }

    /// Parity oracle: the same spread over a corpus built from the *entire* store. Kept as the
    /// ground truth the bounded path is tested against — it deliberately does the whole-store
    /// `list()`+`vectors()` load the production path no longer does.
    #[cfg(test)]
    pub(crate) fn recall_connected_full(
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
        let vectors = self.vectors().unwrap_or_default().into_iter().collect();
        // Full corpus: local tag membership IS global membership, so no hub tags to suppress.
        let neighbors = self.spread_recall(
            &seeds,
            &seed_ids,
            &rows,
            vectors,
            HashSet::new(),
            max_neighbors,
        )?;
        Ok(ConnectedRecall { seeds, neighbors })
    }

    /// Bounded recall with an explicit candidate budget, returning the build stats alongside the
    /// result. Drives the parity and bounds tests; production calls [`Store::recall_connected`],
    /// which fixes the budget at [`crate::store::RECALL_CANDIDATES`].
    #[cfg(test)]
    pub(crate) fn recall_connected_bounded(
        &self,
        text: &str,
        q: &Query,
        actor: &str,
        max_neighbors: usize,
        candidates: usize,
    ) -> Result<(ConnectedRecall, crate::store::BoundedStats), Error> {
        let seeds = self.search(text, q)?;
        let seed_ids: HashSet<String> = seeds.iter().map(|m| m.frontmatter.id.clone()).collect();
        self.log_retrieval(actor, text, &seed_ids.iter().cloned().collect::<Vec<_>>())?;
        if seeds.is_empty() || max_neighbors == 0 {
            return Ok((
                ConnectedRecall {
                    seeds,
                    neighbors: vec![],
                },
                crate::store::BoundedStats::default(),
            ));
        }
        let corpus = self.bounded_corpus(&seeds, text, candidates)?;
        let stats = corpus.stats.clone();
        let neighbors = self.spread_recall(
            &seeds,
            &seed_ids,
            &corpus.rows,
            corpus.vectors,
            corpus.hub_tags,
            max_neighbors,
        )?;
        Ok((ConnectedRecall { seeds, neighbors }, stats))
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

#[cfg(test)]
mod bounded {
    use super::tests::fact;
    use crate::query::Query;
    use crate::store::Store;

    use super::ConnectedRecall;

    /// A well-separated corpus of `n` memories built so the top-k is unambiguous and tie-free:
    ///
    /// * A single "deploy" cluster — one head that keyword-matches `"deploy process"` and links a
    ///   ring of 16 leaves. Each leaf sits alone (its own topic, its own words, no links out), so its
    ///   only activation is the one ref from the head. The leaves are given *distinct* recall counts,
    ///   so their use-boosts — and therefore their final activations — are all different. That makes
    ///   the top-8 a strict, deterministic ordering with no ties for a random `HashMap` iteration to
    ///   scramble differently between two runs.
    /// * The rest is inert filler: unique topics, unique words, no links, no "deploy"/"process" — it
    ///   shares no edge with the cluster, so it can never light up. It exists only to make the store
    ///   big enough that a full scan and a bounded walk visibly diverge in cost.
    fn seed_corpus(n: usize) -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();

        let mut leaves = Vec::new();
        for i in 0..16 {
            let id = store
                .write(&mut fact(
                    &format!("leaf-{i}"),
                    &format!("solitary note about limestone quarry variant {i}"),
                ))
                .unwrap();
            // Distinct recall counts -> distinct use-boosts -> distinct, strictly ordered activations.
            for _ in 0..=i {
                store
                    .log_retrieval("history", "quarry", std::slice::from_ref(&id))
                    .unwrap();
            }
            leaves.push(id);
        }

        let links: String = leaves.iter().map(|id| format!(" [[{id}]]")).collect();
        store
            .write(&mut fact(
                "deploy-head",
                &format!("The deploy process rollout runbook.{links}"),
            ))
            .unwrap();

        for i in 0..n.saturating_sub(17) {
            store
                .write(&mut fact(
                    &format!("filler-{i}"),
                    &format!("miscellaneous gardening note number {i} concerning tulips"),
                ))
                .unwrap();
        }
        (store, dir)
    }

    fn ids_of(r: &ConnectedRecall) -> Vec<String> {
        r.neighbors
            .iter()
            .map(|n| n.memory.frontmatter.id.clone())
            .collect()
    }

    /// Neighbour topics, in rank order. Ids differ between stores (fresh ULIDs), but the cluster's
    /// shape — and therefore which leaf topics win, and in what order — does not.
    fn topics_of(r: &ConnectedRecall) -> Vec<String> {
        r.neighbors
            .iter()
            .map(|n| n.memory.frontmatter.topic.clone().unwrap_or_default())
            .collect()
    }

    /// The core property: recall over the bounded candidate set returns the SAME top-k as recall over
    /// a full-store scan, for a corpus where the top-k is unambiguous.
    #[test]
    fn bounded_recall_matches_full_scan_topk() {
        let (store, _dir) = seed_corpus(2000);
        let q = Query {
            limit: Some(4),
            ..Default::default()
        };
        let full = store
            .recall_connected_full("deploy process", &q, "tester", 8)
            .unwrap();
        let (bounded, _) = store
            .recall_connected_bounded("deploy process", &q, "tester", 8, 128)
            .unwrap();

        assert_eq!(ids_of(&full).len(), 8, "the oracle must fill the top-8");
        assert_eq!(
            ids_of(&full),
            ids_of(&bounded),
            "bounded recall must return the identical top-8 the full scan does"
        );
    }

    /// The bounded path must do `O(candidates + neighbourhood)` work, independent of store size —
    /// never a whole-store scan. Grow the store 10x around the *same* cluster and the corpus the
    /// recall assembles, and the number of retrieval-primitive calls it issues, must not budge.
    #[test]
    fn bounded_recall_work_is_independent_of_store_size() {
        let q = Query {
            limit: Some(4),
            ..Default::default()
        };

        let (small, _small_dir) = seed_corpus(200);
        let (r_small, s_small) = small
            .recall_connected_bounded("deploy process", &q, "tester", 8, 128)
            .unwrap();
        let (large, _large_dir) = seed_corpus(2000);
        let (r_large, s_large) = large
            .recall_connected_bounded("deploy process", &q, "tester", 8, 128)
            .unwrap();

        // Same answer at both sizes: the extra 1800 filler memories are never consulted.
        assert_eq!(topics_of(&r_small), topics_of(&r_large));
        assert_eq!(topics_of(&r_large).len(), 8);

        // The corpus the bounded recall builds is the cluster (head + 16 leaves = 17), NOT the store.
        assert_eq!(s_small.corpus_nodes, 17);
        assert_eq!(
            s_small.corpus_nodes, s_large.corpus_nodes,
            "corpus size must not grow with the store"
        );
        assert!(
            s_large.corpus_nodes < 200,
            "corpus of {} on a 2000-row store proves it is bounded, not a full scan",
            s_large.corpus_nodes
        );

        // The work is fixed by the spread's fan-out, not by n: same expansions, same memoised
        // topic/tag queries, and — the perf guarantee — at most ONE whole-vector load per recall.
        assert_eq!(s_small.neighbor_expansions, s_large.neighbor_expansions);
        assert_eq!(s_small.topic_queries, s_large.topic_queries);
        assert_eq!(s_small.tag_queries, s_large.tag_queries);
        assert_eq!(
            s_small.keyword_candidate_calls,
            s_large.keyword_candidate_calls
        );
        assert_eq!(s_large.full_vector_loads, 1, "one vector load per recall");
        assert_eq!(s_small.full_vector_loads, s_large.full_vector_loads);
    }

    /// A well-separated corpus WITH real (hash) embeddings, built so the top-k is reached ONLY through
    /// semantic edges: a head that keyword-matches the query and a handful of leaves that share the
    /// head's vocabulary (high cosine) but nothing else — no shared topic, no links. If the bounded
    /// walk failed to reconstruct the head's semantic neighbours, those leaves would vanish.
    fn seed_corpus_embedded(n: usize) -> (Store, tempfile::TempDir) {
        use crate::embed::HashEmbedder;
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.set_embedder(Box::new(HashEmbedder::new(256)));

        // Shared vocabulary drives the cosine; the head adds the query words on top.
        for i in 0..4 {
            let id = store
                .write(&mut fact(
                    &format!("sem-{i}"),
                    &format!("alpha beta gamma delta epsilon distinct{i}"),
                ))
                .unwrap();
            for _ in 0..=i {
                store
                    .log_retrieval("history", "vocab", std::slice::from_ref(&id))
                    .unwrap();
            }
        }
        store
            .write(&mut fact(
                "vector-head",
                "deploy process alpha beta gamma delta epsilon",
            ))
            .unwrap();
        for i in 0..n.saturating_sub(5) {
            store
                .write(&mut fact(
                    &format!("filler-{i}"),
                    &format!("miscellaneous gardening note number {i} concerning tulips"),
                ))
                .unwrap();
        }
        (store, dir)
    }

    #[test]
    fn bounded_recall_matches_full_scan_topk_with_vectors() {
        let (store, _dir) = seed_corpus_embedded(1500);
        // Keyword-only seeds so the head is the single, deterministic starting point; the recall's own
        // vector lane and semantic-edge reconstruction still run under the embedder.
        let q = Query {
            limit: Some(1),
            hybrid_weight: Some(0.0),
            ..Default::default()
        };
        let full = store
            .recall_connected_full("deploy process", &q, "tester", 8)
            .unwrap();
        let (bounded, stats) = store
            .recall_connected_bounded("deploy process", &q, "tester", 8, 128)
            .unwrap();

        assert!(
            full.neighbors.iter().all(|n| n
                .memory
                .frontmatter
                .topic
                .as_deref()
                .unwrap_or("")
                .starts_with("sem-")),
            "the neighbourhood must be reached via semantic edges: {:?}",
            topics_of(&full)
        );
        assert_eq!(
            topics_of(&full).len(),
            4,
            "all four semantic leaves light up"
        );
        assert_eq!(
            ids_of(&full),
            ids_of(&bounded),
            "bounded recall must reconstruct the semantic edges the full scan finds"
        );
        assert_eq!(stats.full_vector_loads, 1, "one vector load per recall");
    }

    /// Finding 3 regression guard: a global hub tag (>fan-out members) must be suppressed by the
    /// bounded corpus exactly as the full scan suppresses it. A hub-tag member pulled in via a ref
    /// must NOT gain a spurious tag edge just because only a few of the tag's members are present.
    #[test]
    fn hub_tag_edges_are_suppressed_in_bounded_recall() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        // 15 members share the hub tag "auth" (> RECALL_TAG_FANOUT = 12), each otherwise isolated.
        let mut members = Vec::new();
        for i in 0..15 {
            let mut m = fact(&format!("auth-note-{i}"), &format!("auth detail {i}"));
            m.frontmatter.tags = vec!["auth".into()];
            members.push(store.write(&mut m).unwrap());
        }
        // A seed that keyword-matches, carries the hub tag, and links exactly one member by ref.
        let mut seed = fact("gateway", &format!("widget gateway see [[{}]]", members[0]));
        seed.frontmatter.tags = vec!["auth".into()];
        store.write(&mut seed).unwrap();

        let q = Query {
            limit: Some(1),
            ..Default::default()
        };
        let full = store.recall_connected_full("widget", &q, "t", 8).unwrap();
        let (bounded, _) = store
            .recall_connected_bounded("widget", &q, "t", 8, 128)
            .unwrap();

        let act = |r: &ConnectedRecall, id: &str| {
            r.neighbors
                .iter()
                .find(|n| n.memory.frontmatter.id == id)
                .map(|n| n.activation)
        };
        // The one linked member lights up via its ref (weight 3.0) in both. Without hub-tag
        // suppression the bounded corpus would add a spurious tag edge (weight 1.0) and read 4.0.
        let full_a0 = act(&full, &members[0]).expect("linked member lights up in the full scan");
        let bounded_a0 = act(&bounded, &members[0]).expect("linked member lights up when bounded");
        assert!(
            (full_a0 - bounded_a0).abs() < 1e-6,
            "hub-tag member activation diverged: full {full_a0} vs bounded {bounded_a0}"
        );
        assert_eq!(ids_of(&full), ids_of(&bounded));
    }
}
