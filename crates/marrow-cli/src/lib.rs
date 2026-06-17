//! Command dispatch for the `marrow` CLI.
//!
//! Logic lives in [`run`] (which writes to a caller-supplied sink) so it can be tested
//! without spawning a process. `main.rs` is a thin wrapper.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use marrow_core::seed_anchor;
use marrow_memdocs::{
    CodeAnchor, Frontmatter, Memory, MemoryKind, Provenance, Ref, RefKind, Scope, Status,
};
use marrow_store::{Query, Store};

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
            let anchor = seed_anchor(&args.repo, &args.file, &args.symbol)
                .ok_or_else(|| format!("symbol {} not found in {}", args.symbol, args.file))?;
            let mut memory = build_memory(
                args.kind.into(),
                args.topic,
                args.project,
                args.by,
                vec![],
                args.body,
            );
            memory.frontmatter.refs.push(Ref {
                kind: RefKind::Symbol,
                value: format!("{}::{}", args.file, args.symbol),
                anchor: Some(anchor.fingerprint.clone()),
            });
            memory.frontmatter.code_anchors.push(CodeAnchor {
                file_path: anchor.file_path,
                symbol: anchor.symbol,
                snippet: anchor.snippet,
                fingerprint: anchor.fingerprint,
                norm: anchor.norm,
            });
            let id = store.write(&mut memory).map_err(|e| e.to_string())?;
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
    }
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
