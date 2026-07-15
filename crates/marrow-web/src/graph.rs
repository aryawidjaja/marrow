//! Graph views over the store: memories are neurons, edges are the relationships between them.
//!
//! Two shapes are built here. [`project_graph`] turns a single store into a neuron graph — nodes
//! are memories, edges connect memories that share a topic or a tag, plus any user-drawn links from
//! a small editable overlay. [`hive_graph`] federates every registered project into one brain with
//! a central `core` neuron, a hub per project, and cross-project tag bridges.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use marrow_store::{Edge as Link, EdgeCorpus, EdgeRel, Hub, Store, Trust};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub group: String,
    /// The feature area this memory lives in ("" when unfiled) — the layout groups by this.
    pub area: String,
    /// When this memory was written. Lets the dashboard replay the brain's growth over time.
    pub born: String,
    /// Which agent wrote it, so you can see (and filter by) who knew what.
    pub by: String,
    /// Which model wrote it ("" when the agent didn't say).
    pub model: String,
    pub snippet: String,
    pub degree: usize,
    /// Stable, server-computed content community. The dashboard consumes this directly so every
    /// client sees the same grouping.
    pub community: String,
    /// PageRank-style centrality, normalized to 0..1 within this graph.
    pub importance: f64,
}

#[derive(Serialize, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub review: Vec<ReviewItem>,
}

#[derive(Serialize, Debug)]
pub struct ReviewItem {
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub why: String,
    /// A concrete dashboard action that can make the suggestion disappear or confirm it is valid.
    pub action: String,
    pub nodes: Vec<String>,
}

fn snippet(body: &str, max: usize) -> String {
    let line = body.trim().lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn tags_of(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn ordered(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

/// The neuron graph for one store: memories linked by shared topic, shared tag, related meaning
/// (embeddings), and any links you drew.
pub fn project_graph(store: &Store, root: &Path) -> Graph {
    let rows = store.list().unwrap_or_default();
    let mut nodes = Vec::new();

    for r in &rows {
        if r.status != "active" {
            continue;
        }
        nodes.push(Node {
            id: r.id.clone(),
            label: if r.topic.is_empty() {
                snippet(&r.body, 40)
            } else {
                r.topic.clone()
            },
            kind: r.kind.clone(),
            group: r.area.clone(),
            area: r.area.clone(),
            born: r.created_at.clone(),
            by: r.written_by.clone(),
            model: r.model.clone(),
            snippet: snippet(&r.body, 140),
            degree: 0,
            community: String::new(),
            importance: 0.0,
        });
    }

    let vectors = store.vectors().unwrap_or_default().into_iter().collect();
    let mut links = EdgeCorpus::new(&rows, vectors).graph_edges();

    // Give every area a hub neuron its memories hang off, so an area reads as one connected flower
    // instead of a handful of loose dots. The hub is the area's name, and it's the thing you click
    // to see everything filed there.
    let mut by_area: HashMap<String, Vec<String>> = HashMap::new();
    for r in rows.iter().filter(|r| r.status == "active") {
        if !r.area.is_empty() {
            by_area
                .entry(r.area.clone())
                .or_default()
                .push(r.id.clone());
        }
    }
    let mut area_hubs: Vec<Node> = Vec::new();
    for (area, members) in &by_area {
        let hub_id = format!("area:{area}");
        area_hubs.push(Node {
            id: hub_id.clone(),
            label: area.clone(),
            kind: "area".into(),
            group: area.clone(),
            area: area.clone(),
            born: String::new(),
            by: String::new(),
            model: String::new(),
            snippet: format!("{} memories filed under `{area}`", members.len()),
            degree: 0,
            community: String::new(),
            importance: 0.0,
        });
        for m in members {
            links.push(Link::new(hub_id.clone(), m.clone(), EdgeRel::Area));
        }
    }
    nodes.extend(area_hubs);

    let live: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let overlay = read_overlay(&overlay_path(root));
    for [a, b] in &overlay.links {
        // Skip a saved link whose memory was since deleted, so degree matches the edges drawn.
        if live.contains(a.as_str()) && live.contains(b.as_str()) {
            links.push(Link::new(a.clone(), b.clone(), EdgeRel::User));
        }
    }
    // Drop any link the user explicitly removed from the graph.
    if !overlay.hidden.is_empty() {
        links.retain(|l| {
            !overlay
                .hidden
                .iter()
                .any(|h| same_pair(h, &l.source, &l.target))
        });
    }

    finalize_graph(nodes, links)
}

/// The whole-machine hive: a central `core` neuron, a hub per registered project, each project's
/// memories orbiting its hub, and cross-project bridges by shared *meaning* (mutual nearest
/// neighbours in embedding space) or a rare, specific shared tag — never a generic convention tag.
pub fn hive_graph(hub: &Hub) -> Graph {
    let mut nodes = vec![Node {
        id: "core".into(),
        label: "core".into(),
        kind: "core".into(),
        group: "core".into(),
        area: String::new(),
        born: String::new(),
        by: String::new(),
        model: String::new(),
        snippet: "shared memory about you — the center of the hive".into(),
        degree: 0,
        community: String::new(),
        importance: 0.0,
    }];
    let mut links = Vec::new();
    let mut tag_owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut sem_nodes: Vec<(String, usize, Vec<f32>)> = Vec::new();

    let mut hidden_pairs: Vec<(String, String)> = Vec::new();
    let mut projects = hub.projects();
    if let Ok(core) = hub.core() {
        // The core store is a project too, rooted at the central node.
        projects.insert(
            0,
            marrow_store::Project {
                name: "core".into(),
                root: core.root().to_path_buf(),
            },
        );
    }

    for (i, p) in projects.iter().enumerate() {
        let is_core = p.name == "core";
        // Key hub/memory nodes by registry index, not display name — two projects can share a
        // basename ("app"), and colliding ids would merge their neurons in the client.
        let hub_id = if is_core {
            "core".to_string()
        } else {
            format!("proj:{i}")
        };
        if !is_core {
            nodes.push(Node {
                id: hub_id.clone(),
                label: p.name.clone(),
                kind: "project".into(),
                group: p.name.clone(),
                area: String::new(),
                born: String::new(),
                by: String::new(),
                model: String::new(),
                snippet: format!("project: {}", p.root.display()),
                degree: 0,
                community: String::new(),
                importance: 0.0,
            });
            links.push(Link::new("core", hub_id.clone(), EdgeRel::Hub));
        }
        let Ok(store) = Store::open(&p.root) else {
            continue;
        };
        let vecs: HashMap<String, Vec<f32>> =
            store.vectors().unwrap_or_default().into_iter().collect();
        let mut local_area: HashMap<String, Vec<String>> = HashMap::new();
        let rows = store.list().unwrap_or_default();
        for r in &rows {
            if r.status != "active" {
                continue;
            }
            let node_id = format!("{hub_id}#{}", r.id);
            for t in tags_of(&r.tags) {
                tag_owners
                    .entry(t.clone())
                    .or_default()
                    .push(node_id.clone());
            }
            if let Some(v) = vecs.get(&r.id) {
                sem_nodes.push((node_id.clone(), i, v.clone()));
            }
            nodes.push(Node {
                id: node_id.clone(),
                label: if r.topic.is_empty() {
                    snippet(&r.body, 32)
                } else {
                    r.topic.clone()
                },
                kind: r.kind.clone(),
                group: p.name.clone(),
                area: r.area.clone(),
                born: r.created_at.clone(),
                by: r.written_by.clone(),
                model: r.model.clone(),
                snippet: snippet(&r.body, 140),
                degree: 0,
                community: String::new(),
                importance: 0.0,
            });
            if r.area.is_empty() {
                // Unfiled: hangs straight off the project, so it's never orphaned.
                links.push(Link::new(hub_id.clone(), node_id, EdgeRel::Cluster));
            } else {
                local_area
                    .entry(r.area.clone())
                    .or_default()
                    .push(node_id.clone());
            }
        }
        // The hive's skeleton is core -> project -> area -> memories. Each area gets its own hub
        // neuron (labelled with the area) so you can see WHICH part of the project you're looking
        // at, instead of one undifferentiated blob of the project's colour.
        for (area, members) in &local_area {
            let area_hub = format!("{hub_id}#area:{area}");
            nodes.push(Node {
                id: area_hub.clone(),
                label: area.clone(),
                kind: "area".into(),
                group: p.name.clone(),
                area: area.clone(),
                born: String::new(),
                by: String::new(),
                model: String::new(),
                snippet: format!("{} memories in `{area}` ({})", members.len(), p.name),
                degree: 0,
                community: String::new(),
                importance: 0.0,
            });
            links.push(Link::new(
                hub_id.clone(),
                area_hub.clone(),
                EdgeRel::Cluster,
            ));
            for m in members {
                links.push(Link::new(area_hub.clone(), m.clone(), EdgeRel::Area));
            }
        }
        // Give each project's cluster internal structure — explicit refs, shared topic/tag, and
        // meaning — so it reads as a connected sub-brain, not a bare star around its hub.
        let map = |id: &str| format!("{hub_id}#{id}");

        // Links you drew by hand live in the project's own overlay, so the hive has to read it too.
        // Without this the Link button appears to work here and then nothing shows up.
        let active_ids: std::collections::HashSet<&str> = rows
            .iter()
            .filter(|r| r.status == "active")
            .map(|r| r.id.as_str())
            .collect();
        let overlay = read_overlay(&overlay_path(&p.root));
        for [a, b] in &overlay.links {
            if active_ids.contains(a.as_str()) && active_ids.contains(b.as_str()) {
                links.push(Link::new(map(a), map(b), EdgeRel::User));
            }
        }
        for [a, b] in &overlay.hidden {
            hidden_pairs.push((map(a), map(b)));
        }
        links.extend(
            EdgeCorpus::new(&rows, vecs)
                .graph_edges()
                .into_iter()
                .map(|mut edge| {
                    edge.source = map(&edge.source);
                    edge.target = map(&edge.target);
                    edge
                }),
        );
    }

    // Cross-project links must be meaningful, not coincidental. A shared tag only bridges when it's
    // distinctive (a handful of memories) AND spans projects — generic convention tags like
    // "gotcha" or "frontend" would otherwise wire together unrelated work across every project.
    for members in tag_owners.values() {
        if (2..=TAG_BRIDGE_FANOUT).contains(&members.len()) && spans_projects(members) {
            if let Some((hub, rest)) = members.split_first() {
                links.extend(
                    rest.iter()
                        .map(|target| Link::new(hub, target, EdgeRel::Tag)),
                );
            }
        }
    }
    // The real cross-project bridges: memories whose meaning is close (embedding cosine), across
    // different projects. Silent for projects without embeddings.
    add_hive_semantic(&sem_nodes, &mut links);

    // Drop any link you explicitly removed from the graph.
    if !hidden_pairs.is_empty() {
        links.retain(|l| {
            !hidden_pairs
                .iter()
                .any(|(a, b)| same_pair(&[a.clone(), b.clone()], &l.source, &l.target))
        });
    }

    finalize_graph(nodes, links)
}

/// A shared tag only bridges projects if it's this distinctive or rarer (used by at most this many
/// memories). The data shows nearly every cross-project tag is a generic convention ("gotcha",
/// "frontend"); only an ultra-specific tag shared by two memories is a real link.
const TAG_BRIDGE_FANOUT: usize = 2;
/// Size of each memory's nearest-neighbour set for the mutual-NN meaning test, and a floor so we
/// never bridge in a degenerate low-similarity region.
const HIVE_NN_K: usize = 6;
const HIVE_SEM_FLOOR: f32 = 0.5;

fn project_of(node_id: &str) -> &str {
    node_id.split('#').next().unwrap_or("")
}

fn spans_projects(members: &[String]) -> bool {
    let first = members.first().map(|m| project_of(m));
    members.iter().any(|m| Some(project_of(m)) != first)
}

/// Bridge two memories in different projects only when they are **mutual** nearest neighbours by
/// embedding — each is among the other's closest. This is robust to the high baseline cosine of
/// small embedding models (a fixed threshold links unrelated notes; mutual-NN does not), so
/// unrelated projects get few or no bridges and only genuinely-related memories connect.
fn add_hive_semantic(nodes: &[(String, usize, Vec<f32>)], links: &mut Vec<Link>) {
    let cosine = marrow_store::embed::cosine;
    let nn: Vec<Vec<usize>> = (0..nodes.len())
        .map(|i| {
            let va = &nodes[i].2;
            let mut sims: Vec<(usize, f32)> = (0..nodes.len())
                .filter(|&j| j != i && nodes[j].2.len() == va.len())
                .map(|j| (j, cosine(va, &nodes[j].2)))
                .filter(|(_, s)| *s >= HIVE_SEM_FLOOR)
                .collect();
            sims.sort_by(|a, b| b.1.total_cmp(&a.1));
            sims.into_iter().take(HIVE_NN_K).map(|(j, _)| j).collect()
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for (i, neighbours) in nn.iter().enumerate() {
        for &j in neighbours {
            if nodes[i].1 != nodes[j].1 && nn[j].contains(&i) {
                let (lo, hi) = ordered(&nodes[i].0, &nodes[j].0);
                if seen.insert((lo.clone(), hi.clone())) {
                    let similarity = cosine(&nodes[i].2, &nodes[j].2);
                    links.push(Link::semantic(lo, hi, similarity));
                }
            }
        }
    }
}

fn finalize_graph(mut nodes: Vec<Node>, mut links: Vec<Link>) -> Graph {
    let explicit_pairs: HashSet<(String, String)> = links
        .iter()
        .filter(|link| link.rel != EdgeRel::Semantic)
        .map(|link| ordered(&link.source, &link.target))
        .collect();
    links.retain(|link| {
        link.rel != EdgeRel::Semantic
            || !explicit_pairs.contains(&ordered(&link.source, &link.target))
    });
    let index: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let mut adjacency = vec![Vec::<usize>::new(); nodes.len()];
    let mut content_adjacency = vec![Vec::<usize>::new(); nodes.len()];
    let mut inferred_degree = vec![0usize; nodes.len()];
    for link in &links {
        let (Some(&a), Some(&b)) = (index.get(&link.source), index.get(&link.target)) else {
            continue;
        };
        adjacency[a].push(b);
        adjacency[b].push(a);
        if link.trust != Trust::Structural {
            content_adjacency[a].push(b);
            content_adjacency[b].push(a);
        }
        if link.trust == Trust::Inferred {
            inferred_degree[a] += 1;
            inferred_degree[b] += 1;
        }
    }
    for (node, neighbours) in nodes.iter_mut().zip(&adjacency) {
        node.degree = neighbours.len();
    }

    assign_communities(&mut nodes, &content_adjacency);
    assign_importance(&mut nodes, &adjacency);
    let review = build_review(&nodes, &links, &index, &content_adjacency, &inferred_degree);
    Graph {
        nodes,
        links,
        review,
    }
}

/// Deterministic label propagation over content edges. Community ids are then reindexed by a total
/// order (largest first, member ids as the tie-break), so two clients never invent different names.
fn assign_communities(nodes: &mut [Node], adjacency: &[Vec<usize>]) {
    let mut labels: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|&a, &b| nodes[a].id.cmp(&nodes[b].id));
    for _ in 0..12 {
        let mut changed = false;
        for &i in &order {
            if adjacency[i].is_empty() {
                continue;
            }
            let mut counts: HashMap<String, usize> = HashMap::new();
            for &j in &adjacency[i] {
                *counts.entry(labels[j].clone()).or_default() += 1;
            }
            let best = counts
                .into_iter()
                .max_by(|(la, ca), (lb, cb)| ca.cmp(cb).then_with(|| lb.cmp(la)))
                .map(|(label, _)| label)
                .unwrap_or_else(|| labels[i].clone());
            if best != labels[i] {
                labels[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for (node, label) in nodes.iter().zip(&labels) {
        groups
            .entry(label.clone())
            .or_default()
            .push(node.id.clone());
    }
    let mut stable: Vec<(String, Vec<String>)> = groups.into_iter().collect();
    for (_, members) in &mut stable {
        members.sort();
    }
    stable.sort_by(|(_, a), (_, b)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let remap: HashMap<String, String> = stable
        .iter()
        .enumerate()
        .map(|(i, (old, _))| (old.clone(), format!("c{i}")))
        .collect();
    for (node, old) in nodes.iter_mut().zip(labels) {
        node.community = remap.get(&old).cloned().unwrap_or_default();
    }
}

fn assign_importance(nodes: &mut [Node], adjacency: &[Vec<usize>]) {
    let n = nodes.len();
    if n == 0 {
        return;
    }
    let damping = 0.85;
    let mut rank = vec![1.0 / n as f64; n];
    for _ in 0..24 {
        let dangling: f64 = rank
            .iter()
            .zip(adjacency)
            .filter(|(_, neighbours)| neighbours.is_empty())
            .map(|(r, _)| *r)
            .sum();
        let mut next = vec![(1.0 - damping + damping * dangling) / n as f64; n];
        for (i, neighbours) in adjacency.iter().enumerate() {
            if neighbours.is_empty() {
                continue;
            }
            let share = damping * rank[i] / neighbours.len() as f64;
            for &j in neighbours {
                next[j] += share;
            }
        }
        rank = next;
    }
    let max = rank
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .max(f64::EPSILON);
    for (node, value) in nodes.iter_mut().zip(rank) {
        node.importance = value / max;
    }
}

fn build_review(
    nodes: &[Node],
    links: &[Link],
    index: &HashMap<String, usize>,
    content_adjacency: &[Vec<usize>],
    inferred_degree: &[usize],
) -> Vec<ReviewItem> {
    let is_memory = |n: &Node| !matches!(n.kind.as_str(), "core" | "project" | "area");
    let mut review = Vec::new();

    for link in links.iter().filter(|l| l.trust == Trust::Ambiguous).take(5) {
        review.push(ReviewItem {
            kind: "ambiguous-edge".into(),
            severity: "high".into(),
            title: "Check a weak connection".into(),
            why: format!(
                "Marrow found a {} relationship, but its evidence is below the normal confidence threshold.",
                link.rel
            ),
            action: "Open the highlighted connection. Remove it if the two memories should not be retrieved together.".into(),
            nodes: vec![link.source.clone(), link.target.clone()],
        });
    }

    let mut central: Vec<usize> = (0..nodes.len())
        .filter(|&i| is_memory(&nodes[i]) && inferred_degree[i] >= 2)
        .collect();
    central.sort_by(|&a, &b| {
        nodes[b]
            .importance
            .total_cmp(&nodes[a].importance)
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });
    for &i in central.iter().take(3) {
        review.push(ReviewItem {
            kind: "inferred-hub".into(),
            severity: "medium".into(),
            title: format!("Review inferred links for {}", nodes[i].label),
            why: format!(
                "Marrow inferred {} relationships around this central memory; a person did not create those links.",
                inferred_degree[i]
            ),
            action: "Inspect the visible links. Remove false ones, or draw an explicit link for relationships you want agents to trust.".into(),
            nodes: vec![nodes[i].id.clone()],
        });
    }

    for node in nodes
        .iter()
        .filter(|n| is_memory(n) && n.degree <= 1)
        .take(5)
    {
        review.push(ReviewItem {
            kind: "isolated".into(),
            severity: "low".into(),
            title: format!("Connect {}", node.label),
            why: "This memory has one or fewer connections, so related-memory browsing may not rediscover it.".into(),
            action: "Use Link to connect it to a related memory, or edit its area so it is filed with the right group.".into(),
            nodes: vec![node.id.clone()],
        });
    }

    let mut bridges: Vec<&Link> = links
        .iter()
        .filter(|l| l.trust != Trust::Structural)
        .filter(|l| {
            let (Some(&a), Some(&b)) = (index.get(&l.source), index.get(&l.target)) else {
                return false;
            };
            nodes[a].community != nodes[b].community
        })
        .collect();
    bridges.sort_by(|a, b| {
        let score = |l: &&Link| {
            index
                .get(&l.source)
                .zip(index.get(&l.target))
                .map(|(&x, &y)| nodes[x].importance + nodes[y].importance)
                .unwrap_or(0.0)
        };
        score(b).total_cmp(&score(a))
    });
    for link in bridges.into_iter().take(3) {
        review.push(ReviewItem {
            kind: "bridge".into(),
            severity: "medium".into(),
            title: "Check a bridge between groups".into(),
            why: format!(
                "This {} link is one of the few visible routes between two memory groups.",
                link.rel
            ),
            action: "Open the highlighted connection. Keep it if the groups belong together; remove it if the relationship is misleading.".into(),
            nodes: vec![link.source.clone(), link.target.clone()],
        });
    }

    let mut communities: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, node) in nodes.iter().enumerate().filter(|(_, n)| is_memory(n)) {
        communities.entry(&node.community).or_default().push(i);
    }
    for members in communities.values().filter(|m| m.len() >= 4) {
        let member_set: HashSet<usize> = members.iter().copied().collect();
        let internal: usize = members
            .iter()
            .map(|&i| {
                content_adjacency[i]
                    .iter()
                    .filter(|j| member_set.contains(j))
                    .count()
            })
            .sum::<usize>()
            / 2;
        let possible = members.len() * (members.len() - 1) / 2;
        if possible > 0 && (internal as f64 / possible as f64) < 0.05 {
            let lead = members
                .iter()
                .copied()
                .max_by(|&a, &b| nodes[a].importance.total_cmp(&nodes[b].importance))
                .unwrap_or(members[0]);
            review.push(ReviewItem {
                kind: "low-cohesion".into(),
                severity: "low".into(),
                title: format!("Organize the group around {}", nodes[lead].label),
                why: "This automatically detected group has at least four memories, but fewer than 5% of the possible internal links.".into(),
                action: "Check the members' areas and tags. Add explicit links only where agents should retrieve the memories together.".into(),
                nodes: vec![nodes[lead].id.clone()],
            });
        }
    }
    review.truncate(12);
    review
}

/// Explain the shortest retrieval route between two memories. Structural project/area spokes are
/// deliberately excluded: they organize the drawing but do not prove that two memories are related.
/// Traversal also avoids expanding through a very high-degree content hub, which would otherwise
/// turn many answers into an unhelpful two-hop path.
pub fn path_to_json(graph: &Graph, source: &str, target: &str) -> String {
    if source == target && graph.nodes.iter().any(|n| n.id == source) {
        return json!({ "found": true, "nodes": [source], "links": [] }).to_string();
    }
    let mut adjacency: HashMap<&str, Vec<(&str, usize)>> = HashMap::new();
    for (i, link) in graph.links.iter().enumerate() {
        if link.trust == Trust::Structural {
            continue;
        }
        adjacency
            .entry(&link.source)
            .or_default()
            .push((&link.target, i));
        adjacency
            .entry(&link.target)
            .or_default()
            .push((&link.source, i));
    }
    for neighbours in adjacency.values_mut() {
        neighbours.sort_by(|a, b| a.0.cmp(b.0));
    }
    let mut degrees: Vec<usize> = graph.nodes.iter().map(|n| n.degree).collect();
    degrees.sort_unstable();
    let p99 = degrees
        .get(degrees.len().saturating_sub(1) * 99 / 100)
        .copied()
        .unwrap_or(0);
    let hub_threshold = 50usize.max(p99);
    let degree: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.degree))
        .collect();
    let mut queue = VecDeque::from([source]);
    let mut seen = HashSet::from([source]);
    let mut parent: HashMap<&str, (&str, usize)> = HashMap::new();
    while let Some(at) = queue.pop_front() {
        if at != source && degree.get(at).copied().unwrap_or(0) >= hub_threshold {
            continue;
        }
        for &(next, edge_i) in adjacency.get(at).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(next) {
                parent.insert(next, (at, edge_i));
                if next == target {
                    queue.clear();
                    break;
                }
                queue.push_back(next);
            }
        }
    }
    if !seen.contains(target) {
        return json!({ "found": false, "nodes": [], "links": [] }).to_string();
    }
    let mut node_ids = vec![target];
    let mut edge_ids = Vec::new();
    let mut cursor = target;
    while cursor != source {
        let Some(&(prev, edge_i)) = parent.get(cursor) else {
            return json!({ "found": false, "nodes": [], "links": [] }).to_string();
        };
        edge_ids.push(edge_i);
        node_ids.push(prev);
        cursor = prev;
    }
    node_ids.reverse();
    edge_ids.reverse();
    let route_nodes: Vec<_> = node_ids
        .iter()
        .filter_map(|id| graph.nodes.iter().find(|n| n.id == **id))
        .map(|n| json!({ "id": n.id, "label": n.label, "community": n.community }))
        .collect();
    let route_links: Vec<_> = edge_ids
        .iter()
        .map(|&i| {
            let l = &graph.links[i];
            json!({
                "source": l.source, "target": l.target, "rel": l.rel,
                "trust": l.trust, "confidence": l.confidence
            })
        })
        .collect();
    json!({ "found": true, "nodes": route_nodes, "links": route_links }).to_string()
}

#[derive(Serialize, Deserialize, Default)]
struct Overlay {
    /// Links the user drew by hand.
    #[serde(default)]
    links: Vec<[String; 2]>,
    /// Links (of any kind) the user removed from the graph — a topic/tag/meaning line they don't
    /// want, hidden without touching the memories themselves.
    #[serde(default)]
    hidden: Vec<[String; 2]>,
}

fn overlay_path(root: &Path) -> PathBuf {
    root.join(".marrow").join(".graph").join("links.json")
}

fn read_overlay(path: &Path) -> Overlay {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Overlay>(&s).ok())
        .unwrap_or_default()
}

fn same_pair(l: &[String; 2], a: &str, b: &str) -> bool {
    (l[0] == a && l[1] == b) || (l[0] == b && l[1] == a)
}

fn write_overlay(path: &Path, overlay: &Overlay) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(overlay).unwrap_or_default(),
    )
}

/// Add or remove a user-drawn link in the overlay. Returns the resulting user-link count.
pub fn edit_link(root: &Path, source: &str, target: &str, add: bool) -> std::io::Result<usize> {
    let path = overlay_path(root);
    let mut o = read_overlay(&path);
    if add {
        if !o.links.iter().any(|l| same_pair(l, source, target)) {
            o.links.push([source.to_string(), target.to_string()]);
        }
        // Drawing a link un-hides it, if it was previously hidden.
        o.hidden.retain(|l| !same_pair(l, source, target));
    } else {
        o.links.retain(|l| !same_pair(l, source, target));
    }
    let n = o.links.len();
    write_overlay(&path, &o)?;
    Ok(n)
}

/// Hide (or unhide) any link from the graph without touching the underlying memories. Returns the
/// resulting hidden count.
pub fn edit_hidden(root: &Path, source: &str, target: &str, hide: bool) -> std::io::Result<usize> {
    let path = overlay_path(root);
    let mut o = read_overlay(&path);
    if hide {
        if !o.hidden.iter().any(|l| same_pair(l, source, target)) {
            o.hidden.push([source.to_string(), target.to_string()]);
        }
        // Hiding a hand-drawn link also removes it from the drawn set.
        o.links.retain(|l| !same_pair(l, source, target));
    } else {
        o.hidden.retain(|l| !same_pair(l, source, target));
    }
    let n = o.hidden.len();
    write_overlay(&path, &o)?;
    Ok(n)
}

/// JSON body for the graph endpoints.
pub fn to_json(g: &Graph) -> String {
    json!({ "nodes": g.nodes, "links": g.links, "review": g.review }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};

    fn mem(kind: MemoryKind, topic: &str, body: &str, tags: &[&str]) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind,
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
                tags: tags.iter().map(|s| s.to_string()).collect(),
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: body.into(),
        }
    }

    #[test]
    fn hive_semantic_bridges_only_mutual_cross_project_neighbours() {
        // A (proj 0) and B (proj 1) point the same way — mutual nearest neighbours across projects.
        // C (proj 1) is orthogonal. Expect exactly one bridge, A<->B, and none to C.
        let nodes = vec![
            ("p0#a".to_string(), 0usize, vec![1.0, 0.0, 0.0]),
            ("p1#b".to_string(), 1usize, vec![0.98, 0.20, 0.0]),
            ("p1#c".to_string(), 1usize, vec![0.0, 0.0, 1.0]),
        ];
        let mut links = Vec::new();
        add_hive_semantic(&nodes, &mut links);
        assert_eq!(links.len(), 1, "{links:?}");
        assert_eq!(links[0].rel, EdgeRel::Semantic);
        let pair = ordered(&links[0].source, &links[0].target);
        assert_eq!(pair, ("p0#a".into(), "p1#b".into()));
    }

    #[test]
    fn spans_projects_detects_cross_project_groups() {
        assert!(spans_projects(&["p0#a".into(), "p1#b".into()]));
        assert!(!spans_projects(&["p0#a".into(), "p0#b".into()]));
    }

    #[test]
    fn semantic_edges_link_related_meaning() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.set_embedder(Box::new(marrow_store::HashEmbedder::new(256)));
        // Same body, different topic/no shared tag: only meaning (embedding) can connect them.
        let body =
            "the write cache is invalidated and rebuilt on every persisted mutation of the store";
        store
            .write(&mut mem(MemoryKind::Fact, "alpha", body, &[]))
            .unwrap();
        store
            .write(&mut mem(MemoryKind::Fact, "beta", body, &[]))
            .unwrap();
        store
            .write(&mut mem(
                MemoryKind::Fact,
                "gamma",
                "stripe billing webhooks are signed",
                &[],
            ))
            .unwrap();
        let g = project_graph(&store, dir.path());
        assert!(
            g.links.iter().any(|l| l.rel == EdgeRel::Semantic),
            "expected a related-meaning edge between the two same-meaning memories"
        );
    }

    #[test]
    fn project_graph_links_shared_topic_and_tags_and_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .write(&mut mem(
                MemoryKind::Decision,
                "auth",
                "Use JWT.",
                &["security"],
            ))
            .unwrap();
        store
            .write(&mut mem(
                MemoryKind::Fact,
                "auth",
                "JWT rotates hourly.",
                &[],
            ))
            .unwrap();
        store
            .write(&mut mem(
                MemoryKind::Fact,
                "billing",
                "Stripe webhooks.",
                &["security"],
            ))
            .unwrap();

        let g = project_graph(&store, dir.path());
        assert_eq!(g.nodes.len(), 3);
        // Two share topic "auth"; two share tag "security" — at least two edges.
        assert!(g.links.len() >= 2, "links: {}", g.links.len());

        let ids: Vec<String> = g.nodes.iter().map(|n| n.id.clone()).collect();
        let n = edit_link(dir.path(), &ids[0], &ids[2], true).unwrap();
        assert_eq!(n, 1);
        let g2 = project_graph(&store, dir.path());
        assert!(g2.links.iter().any(|l| l.rel == EdgeRel::User));
        assert_eq!(edit_link(dir.path(), &ids[0], &ids[2], false).unwrap(), 0);
    }

    #[test]
    fn hiding_a_link_removes_it_from_the_graph() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .write(&mut mem(MemoryKind::Decision, "auth", "Use JWT.", &[]))
            .unwrap();
        store
            .write(&mut mem(
                MemoryKind::Fact,
                "auth",
                "JWT rotates hourly.",
                &[],
            ))
            .unwrap();

        let g = project_graph(&store, dir.path());
        let (a, b) = (g.nodes[0].id.clone(), g.nodes[1].id.clone());
        assert!(
            g.links.iter().any(|l| l.rel == EdgeRel::Topic),
            "expected a topic link"
        );

        edit_hidden(dir.path(), &a, &b, true).unwrap();
        assert!(
            project_graph(&store, dir.path()).links.is_empty(),
            "hidden link should be gone"
        );

        edit_hidden(dir.path(), &a, &b, false).unwrap();
        assert!(
            project_graph(&store, dir.path())
                .links
                .iter()
                .any(|l| l.rel == EdgeRel::Topic),
            "unhidden"
        );
    }

    #[test]
    fn graph_intelligence_is_stable_and_paths_explain_their_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        for body in ["first", "second", "third"] {
            store
                .write(&mut mem(MemoryKind::Fact, "shared", body, &[]))
                .unwrap();
        }
        let g = project_graph(&store, dir.path());
        assert!(g.nodes.iter().all(|n| !n.community.is_empty()));
        assert!(g.nodes.iter().all(|n| (0.0..=1.0).contains(&n.importance)));
        assert!(g.links.iter().all(|l| {
            l.rel != EdgeRel::Topic || (l.trust == Trust::Inferred && l.confidence == 0.85)
        }));

        let leaves: Vec<_> = g.nodes.iter().filter(|n| n.degree == 1).collect();
        assert!(leaves.len() >= 2);
        let path: serde_json::Value =
            serde_json::from_str(&path_to_json(&g, &leaves[0].id, &leaves[1].id)).unwrap();
        assert_eq!(path["found"], true);
        assert_eq!(path["nodes"].as_array().unwrap().len(), 3);
        assert!(path["links"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["trust"] == "inferred"));
    }

    #[test]
    fn explained_paths_ignore_structural_area_spokes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let mut a = mem(MemoryKind::Fact, "alpha", "unrelated first fact", &[]);
        let mut b = mem(MemoryKind::Fact, "beta", "unrelated second fact", &[]);
        a.frontmatter.area = Some("shared-area".into());
        b.frontmatter.area = Some("shared-area".into());
        let a_id = store.write(&mut a).unwrap();
        let b_id = store.write(&mut b).unwrap();
        let g = project_graph(&store, dir.path());
        assert!(g.links.iter().any(|link| link.trust == Trust::Structural));
        let path: serde_json::Value =
            serde_json::from_str(&path_to_json(&g, &a_id, &b_id)).unwrap();
        assert_eq!(path["found"], false);
    }

    #[test]
    fn review_queue_surfaces_isolated_memories() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        store
            .write(&mut mem(MemoryKind::Fact, "alone", "no neighbours", &[]))
            .unwrap();
        let g = project_graph(&store, dir.path());
        let isolated = g
            .review
            .iter()
            .find(|item| item.kind == "isolated")
            .expect("isolated suggestion");
        assert!(isolated.title.starts_with("Connect "));
        assert!(isolated.action.contains("Use Link"));
    }
}
