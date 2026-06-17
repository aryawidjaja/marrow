//! Reproducible benchmarks for Marrow.
//!
//! - **ConsolEval** measures the consolidation engine: given good embeddings, does it cluster
//!   the right memories, keep the right survivor, and never merge distinct ones?
//! - **TokenEval** measures retrieval token-efficiency: how much the token budget cuts what a
//!   query returns.
//!
//! Both are deterministic (a lexical hash embedder, fixed corpora) so the numbers reproduce
//! from a clean checkout with no network or model download.

use std::collections::HashSet;

use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::query::estimate_tokens;
use marrow_store::{HashEmbedder, Query, Store};

fn mem(kind: MemoryKind, topic: &str, body: &str, confidence: f64) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(),
            kind,
            status: Status::Active,
            topic: Some(topic.into()),
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: "bench".into(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence,
            decay: None,
            provenance: Provenance {
                written_by: "bench".into(),
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

/// One labeled clustering case: bodies plus the index-groups that should cluster together.
struct Case {
    bodies: Vec<(&'static str, f64)>,
    expected_groups: Vec<Vec<usize>>,
}

fn cases() -> Vec<Case> {
    vec![
        // Exact duplicate.
        Case {
            bodies: vec![
                ("the cache is cleared on write", 1.0),
                ("the cache is cleared on write", 1.0),
            ],
            expected_groups: vec![vec![0, 1]],
        },
        // Same words, reordered (a bag-of-words paraphrase).
        Case {
            bodies: vec![
                ("cache cleared on every write", 1.0),
                ("on every write cache cleared", 1.0),
            ],
            expected_groups: vec![vec![0, 1]],
        },
        // Three identical.
        Case {
            bodies: vec![
                ("rate limit is 100 requests per minute", 1.0),
                ("rate limit is 100 requests per minute", 1.0),
                ("rate limit is 100 requests per minute", 1.0),
            ],
            expected_groups: vec![vec![0, 1, 2]],
        },
        // Genuinely distinct — must NOT merge.
        Case {
            bodies: vec![
                ("authentication uses signed jwt tokens", 1.0),
                ("the dashboard renders in dark mode", 1.0),
            ],
            expected_groups: vec![],
        },
        // Two duplicates plus one distinct.
        Case {
            bodies: vec![
                ("sessions expire after thirty minutes", 1.0),
                ("sessions expire after thirty minutes", 1.0),
                ("payments run through stripe", 1.0),
            ],
            expected_groups: vec![vec![0, 1]],
        },
        // Survivor selection: same body, different confidence — index 1 (higher) should win.
        Case {
            bodies: vec![
                ("the build runs on continuous integration", 0.4),
                ("the build runs on continuous integration", 0.95),
            ],
            expected_groups: vec![vec![0, 1]],
        },
    ]
}

/// Result of the consolidation benchmark.
#[derive(Debug, Clone, Copy)]
pub struct ConsolEval {
    pub cases: usize,
    pub precision: f64,
    pub recall: f64,
    pub false_merges: usize,
    pub survivor_correct: usize,
    pub survivor_total: usize,
}

/// Run ConsolEval over the labeled cases.
pub fn run_consoleval() -> ConsolEval {
    let (mut tp, mut fp, mut fn_) = (0usize, 0usize, 0usize);
    let mut false_merges = 0usize;
    let (mut survivor_correct, mut survivor_total) = (0usize, 0usize);

    for case in cases() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.set_embedder(Box::new(HashEmbedder::new(256)));

        // Write each body; remember which id maps to which case index.
        let mut id_to_index = std::collections::HashMap::new();
        for (i, (body, conf)) in case.bodies.iter().enumerate() {
            let mut m = mem(MemoryKind::Fact, &format!("t{i}"), body, *conf);
            let id = store.write(&mut m).unwrap();
            id_to_index.insert(id, i);
        }

        let report = store.consolidate(dir.path()).unwrap();
        let predicted: Vec<Vec<usize>> = report
            .clusters
            .iter()
            .map(|c| {
                let mut g: Vec<usize> = std::iter::once(&c.keep)
                    .chain(c.others.iter())
                    .filter_map(|id| id_to_index.get(id).copied())
                    .collect();
                g.sort_unstable();
                g
            })
            .collect();

        let expected_pairs = pairs(&case.expected_groups);
        let predicted_pairs = pairs(&predicted);
        tp += expected_pairs.intersection(&predicted_pairs).count();
        fp += predicted_pairs.difference(&expected_pairs).count();
        fn_ += expected_pairs.difference(&predicted_pairs).count();
        false_merges += predicted_pairs.difference(&expected_pairs).count();

        // Survivor check: in any cluster, `keep` should be the highest-confidence member.
        for cluster in &report.clusters {
            survivor_total += 1;
            let group: Vec<usize> = std::iter::once(&cluster.keep)
                .chain(cluster.others.iter())
                .filter_map(|id| id_to_index.get(id).copied())
                .collect();
            let keep_idx = id_to_index.get(&cluster.keep).copied().unwrap();
            let best = group
                .iter()
                .max_by(|a, b| case.bodies[**a].1.partial_cmp(&case.bodies[**b].1).unwrap())
                .copied()
                .unwrap();
            if (case.bodies[keep_idx].1 - case.bodies[best].1).abs() < 1e-9 {
                survivor_correct += 1;
            }
        }
    }

    ConsolEval {
        cases: cases().len(),
        precision: ratio(tp, tp + fp),
        recall: ratio(tp, tp + fn_),
        false_merges,
        survivor_correct,
        survivor_total,
    }
}

/// Result of the token-efficiency benchmark.
#[derive(Debug, Clone, Copy)]
pub struct TokenEval {
    pub memories: usize,
    pub full_tokens: usize,
    pub budget_tokens: usize,
    pub budget: usize,
    pub reduction_pct: f64,
}

/// Run TokenEval: how much a token budget cuts what a broad query returns.
pub fn run_tokeneval() -> TokenEval {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let body = "this is a representative memory body with roughly forty words so that the token \
        estimate per memory is realistic for an agent recalling project knowledge during a long \
        running task that would otherwise overflow the context window entirely and degrade focus";
    let n = 40;
    for i in 0..n {
        let mut m = mem(MemoryKind::Fact, &format!("topic-{i}"), body, 1.0);
        store.write(&mut m).unwrap();
    }
    let full = store.query(&Query::for_project("bench")).unwrap();
    let full_tokens: usize = full.iter().map(|m| estimate_tokens(&m.body)).sum();

    let budget = 500;
    let limited = store
        .query(&Query {
            project_id: Some("bench".into()),
            max_tokens: Some(budget),
            exclude_expired: true,
            ..Query::default()
        })
        .unwrap();
    let budget_tokens: usize = limited.iter().map(|m| estimate_tokens(&m.body)).sum();

    TokenEval {
        memories: n,
        full_tokens,
        budget_tokens,
        budget,
        reduction_pct: if full_tokens == 0 {
            0.0
        } else {
            100.0 * (full_tokens - budget_tokens) as f64 / full_tokens as f64
        },
    }
}

fn pairs(groups: &[Vec<usize>]) -> HashSet<(usize, usize)> {
    let mut out = HashSet::new();
    for g in groups {
        for i in 0..g.len() {
            for j in (i + 1)..g.len() {
                out.insert((g[i].min(g[j]), g[i].max(g[j])));
            }
        }
    }
    out
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        1.0
    } else {
        num as f64 / den as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consoleval_engine_is_correct() {
        let r = run_consoleval();
        assert_eq!(r.precision, 1.0, "must never merge distinct memories");
        assert_eq!(r.recall, 1.0, "must catch every duplicate group");
        assert_eq!(r.false_merges, 0);
        assert_eq!(r.survivor_correct, r.survivor_total);
    }

    #[test]
    fn tokeneval_budget_cuts_results() {
        let r = run_tokeneval();
        assert!(r.budget_tokens <= r.budget + 60, "budget roughly respected");
        assert!(
            r.reduction_pct > 50.0,
            "a small budget should cut a large corpus substantially"
        );
    }
}
