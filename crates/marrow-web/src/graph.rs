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
    pub snippet: String,
    pub degree: usize,
}

#[derive(Serialize)]
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

/// The neuron graph for one store: memories linked by shared topic, shared tag, and user overlay.
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
            group: r.kind.clone(),
            snippet: snippet(&r.body, 140),
            degree: 0,
        });
    }

    let mut links = Vec::new();
    for members in by_topic.values() {
        star(&mut links, members, "topic");
    }
    for members in by_tag.values() {
        star(&mut links, members, "tag");
    }
    let live: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    for [a, b] in read_overlay(&overlay_path(root)) {
        // Skip a saved link whose memory was since deleted, so degree matches the edges drawn.
        if live.contains(a.as_str()) && live.contains(b.as_str()) {
            links.push(Link {
                source: a,
                target: b,
                rel: "user".into(),
            });
        }
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
/// memories orbiting its hub, and cross-project bridges where two projects share a tag.
pub fn hive_graph(hub: &Hub) -> Graph {
    let mut nodes = vec![Node {
        id: "core".into(),
        label: "core".into(),
        kind: "core".into(),
        group: "core".into(),
        snippet: "shared memory about you — the center of the hive".into(),
        degree: 0,
    }];
    let mut links = Vec::new();
    let mut tag_owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut degree: HashMap<String, usize> = HashMap::new();

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
        for r in store.list().unwrap_or_default() {
            if r.status != "active" {
                continue;
            }
            let node_id = format!("{hub_id}#{}", r.id);
            for t in tags_of(&r.tags) {
                tag_owners.entry(t).or_default().push(node_id.clone());
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
                snippet: snippet(&r.body, 140),
                degree: 0,
            });
            links.push(Link {
                source: hub_id.clone(),
                target: node_id,
                rel: "cluster".into(),
            });
        }
    }

    // Bridge memories that share a tag (the cross-project neural links).
    for members in tag_owners.values() {
        star(&mut links, members, "tag");
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

#[derive(Serialize, Deserialize, Default)]
struct Overlay {
    #[serde(default)]
    links: Vec<[String; 2]>,
}

fn overlay_path(root: &Path) -> PathBuf {
    root.join(".marrow").join(".graph").join("links.json")
}

fn read_overlay(path: &Path) -> Vec<[String; 2]> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Overlay>(&s).ok())
        .unwrap_or_default()
        .links
}

/// Add or remove a user-drawn link in the overlay. Returns the resulting link count.
pub fn edit_link(root: &Path, source: &str, target: &str, add: bool) -> std::io::Result<usize> {
    let path = overlay_path(root);
    let mut links = read_overlay(&path);
    let matches =
        |l: &[String; 2]| (l[0] == source && l[1] == target) || (l[0] == target && l[1] == source);
    if add {
        if !links.iter().any(matches) {
            links.push([source.to_string(), target.to_string()]);
        }
    } else {
        links.retain(|l| !matches(l));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(&Overlay {
        links: links.clone(),
    })
    .unwrap_or_default();
    std::fs::write(&path, body)?;
    Ok(links.len())
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
                tags: tags.iter().map(|s| s.to_string()).collect(),
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: body.into(),
        }
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
}
