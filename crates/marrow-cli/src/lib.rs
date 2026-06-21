//! Command dispatch for the `marrow` CLI.
//!
//! Logic lives in [`run`] (which writes to a caller-supplied sink) so it can be tested
//! without spawning a process. `main.rs` is a thin wrapper.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::{ClaimScope, Query, Store};

mod setup;

/// Marrow: a markdown-native memory store for AI agents.
#[derive(Debug, Parser)]
#[command(name = "marrow", version, about)]
pub struct Cli {
    /// Project root containing (or to contain) the `.marrow` store.
    #[arg(long, default_value = ".", global = true)]
    pub root: PathBuf,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Initialize a new store under <root>/.marrow.
    Init,
    /// Write a new memory.
    Add(AddArgs),
    /// Write a memory anchored to a code symbol, so it can be staleness-checked.
    Anchor(AnchorArgs),
    /// Print a memory by id.
    Read { id: String },
    /// Structured query over the index.
    Query(FilterArgs),
    /// Full-text search.
    Search {
        text: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Validate every stored memory against its schema.
    Validate,
    /// List code anchors that no longer match a repo.
    ListStale {
        /// Repo root to check anchors against.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
    /// Supersede an existing memory with a new one.
    Supersede(SupersedeArgs),
    /// Rebuild the index from the markdown files.
    Doctor,
    /// Summary counts for the store.
    Status,
    /// Print the episodic / audit history.
    History {
        /// Show only the most recent N events.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Verify the audit chain is intact.
    Audit,
    /// Append an agent-authored event (observation, correction, note) to the ledger.
    Log {
        /// Event kind, e.g. "observe" or "correct".
        #[arg(long, default_value = "observe")]
        kind: String,
        #[arg(long, default_value = "cli")]
        by: String,
        summary: String,
    },
    /// Detect (or with --apply, perform) consolidation: stale, expired, and duplicates.
    Consolidate {
        /// Repo root to check code anchors against.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Apply changes (merge duplicates, retire expired) instead of only reporting.
        #[arg(long)]
        apply: bool,
    },
    /// Recall memories for a query and record the retrieval (so answers stay traceable).
    Recall {
        text: String,
        #[command(flatten)]
        filter: FilterArgs,
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// Show a memory's provenance: who wrote it, its lineage, and how it's been used.
    Provenance { id: String },
    /// Register an advisory work-claim so parallel sessions don't collide.
    Claim(ClaimArgs),
    /// Release a work-claim by id (otherwise it expires at its TTL).
    Release {
        claim_id: String,
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// List active work-claims; with a scope, only those that would collide.
    Claims {
        #[arg(long = "file")]
        files: Vec<String>,
        #[arg(long = "symbol")]
        symbols: Vec<String>,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long)]
        feature: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Show the most recent activity-stream events across sessions (newest first).
    Activity {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Warm-start a session: announce it, then print active claims + relevant memory.
    Bootstrap {
        goal: String,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value_t = 1500)]
        max_tokens: usize,
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// Record a unit of progress so other sessions see it in real time.
    Progress {
        summary: String,
        #[arg(long, default_value = "cli")]
        session: String,
        #[arg(long = "file")]
        files: Vec<String>,
        #[arg(long, default_value = "cli")]
        by: String,
    },
    /// Wire Marrow into this project + Claude Code in one step: register the MCP server (user
    /// scope), install the auto-capture hooks, and add a short CLAUDE.md guidance block.
    Setup,
}

/// Arguments for `marrow claim`.
#[derive(Debug, clap::Args)]
pub struct ClaimArgs {
    /// What you're about to do, e.g. "refactor auth to async".
    pub intent: String,
    /// Your session id.
    #[arg(long, default_value = "cli")]
    pub session: String,
    /// A file or `dir/*` glob you'll touch (repeatable).
    #[arg(long = "file")]
    pub files: Vec<String>,
    /// A code symbol you'll touch (repeatable).
    #[arg(long = "symbol")]
    pub symbols: Vec<String>,
    #[arg(long)]
    pub topic: Option<String>,
    #[arg(long)]
    pub feature: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    /// Lease length in seconds (default 15 minutes). Leases renew automatically on progress, so
    /// active work keeps its claim while abandoned claims free up quickly.
    #[arg(long, default_value_t = 900)]
    pub ttl_secs: i64,
    #[arg(long, default_value = "cli")]
    pub by: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum KindArg {
    Fact,
    Decision,
    Entity,
    Session,
    Skill,
}

impl From<KindArg> for MemoryKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Fact => MemoryKind::Fact,
            KindArg::Decision => MemoryKind::Decision,
            KindArg::Entity => MemoryKind::Entity,
            KindArg::Session => MemoryKind::Session,
            KindArg::Skill => MemoryKind::Skill,
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct AddArgs {
    #[arg(long, value_enum)]
    pub kind: KindArg,
    #[arg(long)]
    pub topic: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    /// Who is writing this memory (provenance).
    #[arg(long, default_value = "cli")]
    pub by: String,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// The memory body.
    pub body: String,
}

#[derive(Debug, clap::Args)]
pub struct AnchorArgs {
    #[arg(long, value_enum)]
    pub kind: KindArg,
    #[arg(long)]
    pub topic: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long, default_value = "cli")]
    pub by: String,
    /// Repo root the symbol lives in.
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
    /// File containing the symbol, relative to --repo.
    #[arg(long)]
    pub file: String,
    /// Qualified symbol name, e.g. "Foo::bar".
    #[arg(long)]
    pub symbol: String,
    pub body: String,
}

#[derive(Debug, clap::Args)]
pub struct SupersedeArgs {
    pub old_id: String,
    #[arg(long, value_enum)]
    pub kind: KindArg,
    #[arg(long)]
    pub topic: Option<String>,
    #[arg(long, default_value = "cli")]
    pub by: String,
    pub body: String,
}

#[derive(Debug, Default, clap::Args)]
pub struct FilterArgs {
    #[arg(long, value_enum)]
    pub kind: Option<KindArg>,
    #[arg(long)]
    pub topic: Option<String>,
    #[arg(long)]
    pub project: Option<String>,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub min_confidence: Option<f64>,
    #[arg(long)]
    pub max_tokens: Option<usize>,
    #[arg(long)]
    pub limit: Option<usize>,
    /// Include expired memories.
    #[arg(long)]
    pub include_expired: bool,
    /// Hybrid search weight: 0 = keyword only, 1 = semantic only.
    #[arg(long)]
    pub weight: Option<f64>,
}

impl FilterArgs {
    fn to_query(&self) -> Query {
        Query {
            kind: self.kind.map(Into::into),
            status: Some(Status::Active),
            topic: self.topic.clone(),
            project_id: self.project.clone(),
            tag: self.tag.clone(),
            min_confidence: self.min_confidence,
            max_tokens: self.max_tokens,
            limit: self.limit,
            exclude_expired: !self.include_expired,
            hybrid_weight: self.weight,
            ..Query::default()
        }
    }
}

/// Run a parsed CLI command, writing user output to `out`.
pub fn run(cli: Cli, out: &mut impl Write) -> Result<(), String> {
    match cli.cmd {
        Cmd::Init => {
            Store::init(&cli.root).map_err(|e| e.to_string())?;
            writeln!(
                out,
                "Initialized Marrow store at {}/.marrow",
                cli.root.display()
            )
            .ok();
            Ok(())
        }
        Cmd::Add(args) => {
            let store = open(&cli.root)?;
            let mut memory = build_memory(
                args.kind.into(),
                args.topic,
                args.project,
                args.by,
                args.tags,
                args.body,
            );
            let id = store.write(&mut memory).map_err(|e| e.to_string())?;
            writeln!(out, "{id}").ok();
            Ok(())
        }
        Cmd::Anchor(args) => {
            let store = open(&cli.root)?;
            let mut memory = build_memory(
                args.kind.into(),
                args.topic,
                args.project,
                args.by,
                vec![],
                args.body,
            );
            let id = store
                .write_anchored(&args.repo, &args.file, &args.symbol, &mut memory)
                .map_err(|e| e.to_string())?;
            writeln!(out, "{id}").ok();
            Ok(())
        }
        Cmd::Read { id } => {
            let store = open(&cli.root)?;
            match store.read(&id).map_err(|e| e.to_string())? {
                Some(m) => {
                    write!(out, "{}", marrow_memdocs::to_markdown(&m)).ok();
                    Ok(())
                }
                None => Err(format!("no memory with id {id}")),
            }
        }
        Cmd::Query(filter) => {
            let store = open(&cli.root)?;
            let hits = store.query(&filter.to_query()).map_err(|e| e.to_string())?;
            print_memories(&hits, out);
            Ok(())
        }
        Cmd::Search { text, filter } => {
            let store = open(&cli.root)?;
            let hits = store
                .search(&text, &filter.to_query())
                .map_err(|e| e.to_string())?;
            print_memories(&hits, out);
            Ok(())
        }
        Cmd::Validate => {
            let store = open(&cli.root)?;
            let mut problems = 0;
            for row in store.list().map_err(|e| e.to_string())? {
                if let Some(m) = store.read(&row.id).map_err(|e| e.to_string())? {
                    if let Err(violations) = marrow_memdocs::validate(&m) {
                        problems += violations.len();
                        for v in violations {
                            writeln!(out, "{}: {v}", row.id).ok();
                        }
                    }
                }
            }
            writeln!(out, "{problems} problem(s)").ok();
            if problems == 0 {
                Ok(())
            } else {
                Err(format!("{problems} validation problem(s)"))
            }
        }
        Cmd::ListStale { repo } => {
            let store = open(&cli.root)?;
            let stale = store.list_stale(&repo).map_err(|e| e.to_string())?;
            for hit in &stale {
                let reloc = hit
                    .relocated_to
                    .as_deref()
                    .map(|r| format!(" -> {r}"))
                    .unwrap_or_default();
                writeln!(
                    out,
                    "STALE {} {} ({}){reloc}",
                    hit.memory_id, hit.symbol, hit.file_path
                )
                .ok();
            }
            writeln!(out, "{} stale anchor(s)", stale.len()).ok();
            Ok(())
        }
        Cmd::Supersede(args) => {
            let store = open(&cli.root)?;
            let mut memory = build_memory(
                args.kind.into(),
                args.topic,
                None,
                args.by,
                vec![],
                args.body,
            );
            let id = store
                .supersede(&args.old_id, &mut memory)
                .map_err(|e| e.to_string())?;
            writeln!(out, "{id}").ok();
            Ok(())
        }
        Cmd::Doctor => {
            let store = open(&cli.root)?;
            let n = store.reindex().map_err(|e| e.to_string())?;
            let embedded = store.reembed().map_err(|e| e.to_string())?;
            writeln!(out, "Reindexed {n} mem(s); embedded {embedded}").ok();
            Ok(())
        }
        Cmd::Status => {
            let store = open(&cli.root)?;
            let rows = store.list().map_err(|e| e.to_string())?;
            writeln!(out, "total: {}", rows.len()).ok();
            for kind in ["fact", "decision", "entity", "session", "skill"] {
                let n = rows.iter().filter(|r| r.kind == kind).count();
                if n > 0 {
                    writeln!(out, "  {kind}: {n}").ok();
                }
            }
            Ok(())
        }
        Cmd::History { limit } => {
            let store = open(&cli.root)?;
            let events = store.history().map_err(|e| e.to_string())?;
            let start = limit.map(|n| events.len().saturating_sub(n)).unwrap_or(0);
            for e in &events[start..] {
                let mem = e.memory_id.as_deref().unwrap_or("-");
                writeln!(
                    out,
                    "{}  {}  {}  {}  {} [{mem}]",
                    e.seq, e.ts, e.kind, e.actor, e.summary
                )
                .ok();
            }
            writeln!(out, "{} event(s)", events.len()).ok();
            Ok(())
        }
        Cmd::Audit => {
            let store = open(&cli.root)?;
            match store.verify_log() {
                Ok(()) => {
                    let n = store.history().map_err(|e| e.to_string())?.len();
                    writeln!(out, "audit ok: {n} event(s), chain intact").ok();
                    Ok(())
                }
                Err(seq) => Err(format!("audit chain broken at seq {seq}")),
            }
        }
        Cmd::Log { kind, by, summary } => {
            let store = open(&cli.root)?;
            store
                .log_event(&kind, &by, &summary)
                .map_err(|e| e.to_string())?;
            writeln!(out, "logged").ok();
            Ok(())
        }
        Cmd::Consolidate { repo, apply } => {
            let store = open(&cli.root)?;
            if apply {
                let o = store.consolidate_apply(&repo).map_err(|e| e.to_string())?;
                writeln!(
                    out,
                    "applied: {} deprecated, {} merged, {} conflicts resolved",
                    o.deprecated, o.merged, o.conflicts_resolved
                )
                .ok();
            } else {
                let r = store.consolidate(&repo).map_err(|e| e.to_string())?;
                let related: usize = r.clusters.iter().map(|c| c.others.len()).sum();
                writeln!(
                    out,
                    "stale: {}, expired: {}, related memories: {} (in {} cluster(s))",
                    r.stale.len(),
                    r.expired.len(),
                    related,
                    r.clusters.len(),
                )
                .ok();
            }
            Ok(())
        }
        Cmd::Recall { text, filter, by } => {
            let store = open(&cli.root)?;
            let hits = store
                .recall(&text, &filter.to_query(), &by)
                .map_err(|e| e.to_string())?;
            print_memories(&hits, out);
            Ok(())
        }
        Cmd::Provenance { id } => {
            let store = open(&cli.root)?;
            match store.provenance(&id).map_err(|e| e.to_string())? {
                Some(t) => {
                    writeln!(out, "memory {} — written by {}", t.id, t.written_by).ok();
                    if !t.sources.is_empty() {
                        writeln!(out, "  sources: {}", t.sources.join(", ")).ok();
                    }
                    for r in &t.supersedes {
                        writeln!(
                            out,
                            "  supersedes {} [{}] {}",
                            r.id,
                            r.kind,
                            r.topic.as_deref().unwrap_or("-")
                        )
                        .ok();
                    }
                    for r in &t.superseded_by {
                        writeln!(out, "  superseded by {} [{}]", r.id, r.kind).ok();
                    }
                    writeln!(out, "  history:").ok();
                    for e in &t.events {
                        writeln!(out, "    #{} {} {} — {}", e.seq, e.ts, e.kind, e.summary).ok();
                    }
                    Ok(())
                }
                None => Err(format!("no memory with id {id}")),
            }
        }
        Cmd::Claim(a) => {
            let store = open(&cli.root)?;
            let scope = ClaimScope {
                files: a.files,
                symbols: a.symbols,
                topic: a.topic,
                feature: a.feature,
                project_id: a.project.unwrap_or_default(),
            };
            let c = store
                .claim(&a.session, &a.by, scope, &a.intent, a.ttl_secs)
                .map_err(|e| e.to_string())?;
            writeln!(out, "{}", c.id).ok();
            Ok(())
        }
        Cmd::Release { claim_id, by } => {
            let store = open(&cli.root)?;
            store.release(&claim_id, &by).map_err(|e| e.to_string())?;
            writeln!(out, "released {claim_id}").ok();
            Ok(())
        }
        Cmd::Claims {
            files,
            symbols,
            topic,
            feature,
            project,
        } => {
            let store = open(&cli.root)?;
            let scope = ClaimScope {
                files,
                symbols,
                topic,
                feature,
                project_id: project.unwrap_or_default(),
            };
            let scoped = !scope.files.is_empty()
                || !scope.symbols.is_empty()
                || scope.topic.is_some()
                || scope.feature.is_some();
            let found = if scoped {
                store.claims_overlapping(&scope)
            } else {
                store.active_claims()
            }
            .map_err(|e| e.to_string())?;
            for c in &found {
                writeln!(out, "{}  [{}]  {}", c.id, c.session_id, c.intent).ok();
            }
            writeln!(out, "{} active claim(s)", found.len()).ok();
            Ok(())
        }
        Cmd::Activity { limit } => {
            let store = open(&cli.root)?;
            let events = store.activity(limit).map_err(|e| e.to_string())?;
            for e in &events {
                writeln!(
                    out,
                    "{}  {}  {}  {} — {}",
                    e.seq, e.ts, e.kind, e.actor, e.summary
                )
                .ok();
            }
            writeln!(out, "{} event(s)", events.len()).ok();
            Ok(())
        }
        Cmd::Bootstrap {
            goal,
            project,
            max_tokens,
            by,
        } => {
            let store = open(&cli.root)?;
            let brief = store
                .bootstrap(&goal, &project, &by, max_tokens)
                .map_err(|e| e.to_string())?;
            writeln!(out, "goal: {}", brief.goal).ok();
            writeln!(out, "active claims ({}):", brief.active_claims.len()).ok();
            for c in &brief.active_claims {
                writeln!(out, "  {} — {} [{}]", c.id, c.intent, c.session_id).ok();
            }
            writeln!(out, "recent decisions ({}):", brief.recent_decisions.len()).ok();
            for m in &brief.recent_decisions {
                writeln!(out, "  {} — {}", m.frontmatter.id, first_line(&m.body)).ok();
            }
            writeln!(out, "relevant memories ({}):", brief.relevant.len()).ok();
            for m in &brief.relevant {
                writeln!(out, "  {} — {}", m.frontmatter.id, first_line(&m.body)).ok();
            }
            Ok(())
        }
        Cmd::Progress {
            summary,
            session,
            files,
            by,
        } => {
            let store = open(&cli.root)?;
            store
                .progress(&session, &by, &summary, &files)
                .map_err(|e| e.to_string())?;
            writeln!(out, "recorded").ok();
            Ok(())
        }
        Cmd::Setup => setup::run(&cli.root, out),
    }
}

fn first_line(body: &str) -> &str {
    body.trim().lines().next().unwrap_or("")
}

fn open(root: &std::path::Path) -> Result<Store, String> {
    Store::open(root).map_err(|e| e.to_string())
}

fn build_memory(
    kind: MemoryKind,
    topic: Option<String>,
    project: Option<String>,
    by: String,
    tags: Vec<String>,
    body: String,
) -> Memory {
    Memory {
        frontmatter: Frontmatter {
            id: String::new(),
            kind,
            status: Status::Active,
            topic,
            scope: Scope {
                user_id: None,
                agent_id: None,
                project_id: project.unwrap_or_default(),
                org_id: None,
            },
            refs: vec![],
            code_anchors: vec![],
            confidence: 1.0,
            decay: None,
            provenance: Provenance {
                written_by: by,
                session_id: None,
                sources: vec![],
            },
            supersedes: vec![],
            tags,
            created_at: String::new(),
            updated_at: String::new(),
            hmac: None,
        },
        body,
    }
}

fn print_memories(hits: &[Memory], out: &mut impl Write) {
    for m in hits {
        let topic = m.frontmatter.topic.as_deref().unwrap_or("-");
        let first = m.body.lines().next().unwrap_or("").trim();
        writeln!(out, "{}  [{}]  {first}", m.frontmatter.id, topic).ok();
    }
    writeln!(out, "{} result(s)", hits.len()).ok();
}
