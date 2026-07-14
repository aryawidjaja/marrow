//! Graph views over the store: memories are neurons, edges are the relationships between them.
//!
//! Two shapes are built here. [`project_graph`] turns a single store into a neuron graph — nodes
//! are memories, edges connect memories that share a topic or a tag, plus any user-drawn links from
//! a small editable overlay. [`hive_graph`] federates every registered project into one brain with
//! a central `core` neuron, a hub per project, and cross-project tag bridges.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use marrow_store::{Hub, Store};
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
}

#[derive(Serialize, Debug)]
pub struct Link {
    pub source: String,
    pub target: String,
    pub rel: String,
}

#[derive(Serialize, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
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

/// A tag shared by more than this many memories in one project is a broad convention, not a
/// specific relationship — it would draw a giant noise-star, so it doesn't create edges.
const INTRA_TAG_FANOUT: usize = 7;

/// Extract `[[target]]` wiki-links from text — the explicit references an agent wrote between
/// memories. `target` is either a memory id or a topic slug.
fn wiki_refs(text: &str) -> Vec<String> {
    text.match_indices("[[")
        .filter_map(|(i, _)| {
            let rest = &text[i + 2..];
            rest.find("]]").map(|j| rest[..j].trim().to_string())
        })
        .filter(|s| !s.is_empty() && s.len() <= 48)
        .collect()
}

/// Explicit `[[id]]`/`[[topic]]` links between memories — the strongest, agent-authored edges.
fn ref_edges(rows: &[marrow_store::index::IndexRow], id_of: &impl Fn(&str) -> String) -> Vec<Link> {
    let active = || rows.iter().filter(|r| r.status == "active");
    let ids: std::collections::HashSet<&str> = active().map(|r| r.id.as_str()).collect();
    let topic_to_id: HashMap<&str, &str> = active()
        .filter(|r| !r.topic.is_empty())
        .map(|r| (r.topic.as_str(), r.id.as_str()))
        .collect();
    let mut links = Vec::new();
    for r in active() {
        for rf in wiki_refs(&r.body).into_iter().chain(wiki_refs(&r.topic)) {
            let target = if ids.contains(rf.as_str()) {
                Some(rf)
            } else {
                topic_to_id.get(rf.as_str()).map(|s| s.to_string())
            };
            if let Some(t) = target {
                if t != r.id {
                    links.push(Link {
                        source: id_of(&r.id),
                        target: id_of(&t),
                        rel: "ref".into(),
                    });
                }
            }
        }
    }
    links
}

/// Connect every node in a group to the group's first node — a star, so a shared topic or tag
/// forms a cluster without an O(n²) clique.
fn star(links: &mut Vec<Link>, members: &[String], rel: &str) {
    if let Some((hub, rest)) = members.split_first() {
        for m in rest {
            links.push(Link {
                source: hub.clone(),
                target: m.clone(),
                rel: rel.into(),
            });
        }
    }
}

/// How many meaning-neighbours each memory keeps, and how close they must be. bge-m3 cosine puts
/// genuinely related notes around 0.5–0.75; a top-k cap keeps the graph readable, not a hairball.
const SEMANTIC_TOP_K: usize = 3;
const SEMANTIC_MIN_SIM: f32 = 0.55;

fn ordered(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    }
}

/// Add "related meaning" edges from embedding cosine, skipping any pair a topic/tag/user link
/// already connects so the same two neurons aren't joined twice.
fn add_semantic(store: &Store, links: &mut Vec<Link>, node_id: impl Fn(&str) -> String) {
    let existing: std::collections::HashSet<(String, String)> = links
        .iter()
        .map(|l| ordered(&l.source, &l.target))
        .collect();
    for (a, b, _sim) in store
        .related(SEMANTIC_TOP_K, SEMANTIC_MIN_SIM)
        .unwrap_or_default()
    {
        let (na, nb) = (node_id(&a), node_id(&b));
        if existing.contains(&ordered(&na, &nb)) {
            continue;
        }
        links.push(Link {
            source: na,
            target: nb,
            rel: "semantic".into(),
        });
    }
}

/// The neuron graph for one store: memories linked by shared topic, shared tag, related meaning
/// (embeddings), and any links you drew.
pub fn project_graph(store: &Store, root: &Path) -> Graph {
    let rows = store.list().unwrap_or_default();
    let mut by_topic: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tag: HashMap<String, Vec<String>> = HashMap::new();
    let mut nodes = Vec::new();
    let mut degree: HashMap<String, usize> = HashMap::new();

    for r in &rows {
        if r.status != "active" {
            continue;
        }
        if !r.topic.is_empty() {
            by_topic
                .entry(r.topic.clone())
                .or_default()
                .push(r.id.clone());
        }
        for t in tags_of(&r.tags) {
            by_tag.entry(t).or_default().push(r.id.clone());
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
        });
    }

    let mut links = ref_edges(&rows, &|id: &str| id.to_string());

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
        });
        for m in members {
            links.push(Link {
                source: hub_id.clone(),
                target: m.clone(),
                rel: "area".into(),
            });
        }
    }
    nodes.extend(area_hubs);

    for members in by_topic.values() {
        star(&mut links, members, "topic");
    }
    for members in by_tag.values() {
        if members.len() <= INTRA_TAG_FANOUT {
            star(&mut links, members, "tag");
        }
    }
    let live: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let overlay = read_overlay(&overlay_path(root));
    for [a, b] in &overlay.links {
        // Skip a saved link whose memory was since deleted, so degree matches the edges drawn.
        if live.contains(a.as_str()) && live.contains(b.as_str()) {
            links.push(Link {
                source: a.clone(),
                target: b.clone(),
                rel: "user".into(),
            });
        }
    }
    add_semantic(store, &mut links, |id| id.to_string());
    // Drop any link the user explicitly removed from the graph.
    if !overlay.hidden.is_empty() {
        links.retain(|l| {
            !overlay
                .hidden
                .iter()
                .any(|h| same_pair(h, &l.source, &l.target))
        });
    }

    for l in &links {
        *degree.entry(l.source.clone()).or_default() += 1;
        *degree.entry(l.target.clone()).or_default() += 1;
    }
    for n in &mut nodes {
        n.degree = degree.get(&n.id).copied().unwrap_or(0);
    }
    Graph { nodes, links }
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
    }];
    let mut links = Vec::new();
    let mut tag_owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut sem_nodes: Vec<(String, usize, Vec<f32>)> = Vec::new();
    let mut degree: HashMap<String, usize> = HashMap::new();

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
            });
            links.push(Link {
                source: "core".into(),
                target: hub_id.clone(),
                rel: "hub".into(),
            });
        }
        let Ok(store) = Store::open(&p.root) else {
            continue;
        };
        let vecs: HashMap<String, Vec<f32>> =
            store.vectors().unwrap_or_default().into_iter().collect();
        let mut local_topic: HashMap<String, Vec<String>> = HashMap::new();
        let mut local_tag: HashMap<String, Vec<String>> = HashMap::new();
        let mut local_area: HashMap<String, Vec<String>> = HashMap::new();
        let rows = store.list().unwrap_or_default();
        for r in &rows {
            if r.status != "active" {
                continue;
            }
            let node_id = format!("{hub_id}#{}", r.id);
            if !r.topic.is_empty() {
                local_topic
                    .entry(r.topic.clone())
                    .or_default()
                    .push(node_id.clone());
            }
            for t in tags_of(&r.tags) {
                tag_owners
                    .entry(t.clone())
                    .or_default()
                    .push(node_id.clone());
                local_tag.entry(t).or_default().push(node_id.clone());
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
            });
            if r.area.is_empty() {
                // Unfiled: hangs straight off the project, so it's never orphaned.
                links.push(Link {
                    source: hub_id.clone(),
                    target: node_id,
                    rel: "cluster".into(),
                });
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
            });
            links.push(Link {
                source: hub_id.clone(),
                target: area_hub.clone(),
                rel: "cluster".into(),
            });
            for m in members {
                links.push(Link {
                    source: area_hub.clone(),
                    target: m.clone(),
                    rel: "area".into(),
                });
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
                links.push(Link {
                    source: map(a),
                    target: map(b),
                    rel: "user".into(),
                });
            }
        }
        for [a, b] in &overlay.hidden {
            hidden_pairs.push((map(a), map(b)));
        }
        links.extend(ref_edges(&rows, &map));
        for members in local_topic.values() {
            star(&mut links, members, "topic");
        }
        for members in local_tag.values() {
            if members.len() <= INTRA_TAG_FANOUT {
                star(&mut links, members, "tag");
            }
        }
        add_semantic(&store, &mut links, map);
    }

    // Cross-project links must be meaningful, not coincidental. A shared tag only bridges when it's
    // distinctive (a handful of memories) AND spans projects — generic convention tags like
    // "gotcha" or "frontend" would otherwise wire together unrelated work across every project.
    for members in tag_owners.values() {
        if (2..=TAG_BRIDGE_FANOUT).contains(&members.len()) && spans_projects(members) {
            star(&mut links, members, "tag");
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

    for l in &links {
        *degree.entry(l.source.clone()).or_default() += 1;
        *degree.entry(l.target.clone()).or_default() += 1;
    }
    for n in &mut nodes {
        n.degree = degree.get(&n.id).copied().unwrap_or(0);
    }
    Graph { nodes, links }
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
                    links.push(Link {
                        source: lo,
                        target: hi,
                        rel: "semantic".into(),
                    });
                }
            }
        }
    }
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
    json!({ "nodes": g.nodes, "links": g.links }).to_string()
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
        assert_eq!(links[0].rel, "semantic");
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
            g.links.iter().any(|l| l.rel == "semantic"),
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
        assert!(g2.links.iter().any(|l| l.rel == "user"));
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
            g.links.iter().any(|l| l.rel == "topic"),
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
                .any(|l| l.rel == "topic"),
            "unhidden"
        );
    }
}
