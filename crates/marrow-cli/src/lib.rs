//! Command dispatch for the `marrow` CLI.
//!
//! Logic lives in [`run`] (which writes to a caller-supplied sink) so it can be tested
//! without spawning a process. `main.rs` is a thin wrapper.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};
use marrow_store::{knowledge_docs, ClaimScope, Query, Store};

mod setup;

/// Marrow: a markdown-native memory store for AI agents.
#[derive(Debug, Parser)]
#[command(name = "marrow", version, about)]
pub struct Cli {
    /// Project root containing (or to contain) the `.marrow` store.
    #[arg(long, default_value = ".", global = true)]
    pub root: PathBuf,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
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
        /// Apply only if enough new memories have accumulated since the last pass (otherwise a
        /// no-op). Used for hands-free auto-consolidation.
        #[arg(long)]
        if_due: bool,
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
    /// Release a work-claim by id, or with --session release every active claim that session holds.
    Release {
        /// Claim id to release (omit when using --session).
        claim_id: Option<String>,
        /// Release ALL active claims held by this session id (used when a session goes idle).
        #[arg(long)]
        session: Option<String>,
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
    Setup {
        /// Install the hooks once at the user level (~/.claude) so every project is hands-free,
        /// instead of just this project's .claude.
        #[arg(long)]
        global: bool,
    },
    /// List the project's existing knowledge docs and tell the agent how to seed memory from
    /// them — a one-time onboarding step for an existing repo.
    Ingest,
    /// Update marrow to the latest version (detects brew/cargo/curl) and refresh hooks + MCP.
    Upgrade,
    /// Show what other sessions have done since this session last checked (powers the hive-mind
    /// awareness hook). Prints nothing when no other session is active.
    Watch {
        #[arg(long, default_value = "cli")]
        session: String,
    },
    /// Choose the search backend: `none`/`hash` (keyword, default) or `fastembed`/`http`
    /// (semantic). Semantic needs a binary built with the matching feature.
    Embed {
        /// none | hash | fastembed | http
        provider: String,
        /// Endpoint for the `http` provider.
        #[arg(long)]
        url: Option<String>,
    },
    /// Cross-project hive: federate every registered brain on this machine into one second brain.
    Hub {
        #[command(subcommand)]
        cmd: HubCmd,
    },
    /// Share THIS project to a gateway "space" so its memory is shared with other machines or
    /// teammates. Every other project stays local and private. Machines using the same gateway +
    /// space + token share one brain. Nothing local is deleted.
    Share {
        /// Gateway base URL, e.g. https://team.fly.dev.
        #[arg(long)]
        gateway: String,
        /// The space (shared-brain key) on the gateway. Same name on every machine = same brain.
        #[arg(long)]
        space: String,
        /// Bearer token / API key for the gateway.
        #[arg(long)]
        token: Option<String>,
    },
    /// Stop sharing this project — make it local and private again. Nothing is deleted.
    Unshare,
}

/// Subcommands for the cross-project hub.
#[derive(Debug, Subcommand)]
pub enum HubCmd {
    /// Register a project into the hive so its knowledge joins cross-project recall (defaults to --root).
    Register {
        /// Display name for the project (defaults to the directory name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Remove a project from the hive by name or path.
    Forget { key: String },
    /// List the projects currently in the hive.
    List,
    /// Recall across every registered brain — what does the whole hive know about this?
    Recall {
        text: String,
        #[arg(long, default_value_t = 8)]
        limit: usize,
    },
    /// What agents in other projects are doing right now (newest first).
    Activity {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Save a shared core memory (about you, or cross-project) that every project can see.
    Remember {
        body: String,
        #[arg(long)]
        topic: Option<String>,
    },
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
    let Some(cmd) = cli.cmd else {
        return welcome(out);
    };
    match cmd {
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
            if let Some(remote) = marrow_store::SharedRemote::load(&cli.root) {
                writeln!(
                    out,
                    "sharing: shared to space '{}' on {} (agents here use that shared brain; counts below are the local cache). `marrow unshare` to go local.",
                    remote.space, remote.url
                )
                .ok();
            } else {
                writeln!(out, "sharing: local (private to this machine).").ok();
            }
            let store = open(&cli.root)?;
            let rows = store.list().map_err(|e| e.to_string())?;
            writeln!(out, "total: {}", rows.len()).ok();
            for kind in ["fact", "decision", "entity", "session", "skill"] {
                let n = rows.iter().filter(|r| r.kind == kind).count();
                if n > 0 {
                    writeln!(out, "  {kind}: {n}").ok();
                }
            }
            match store.embedding_provider() {
                "fastembed" | "http" => writeln!(out, "search: semantic").ok(),
                _ => writeln!(
                    out,
                    "search: keyword — enable smarter semantic recall with `marrow embed fastembed` (see README)."
                )
                .ok(),
            };
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
        Cmd::Consolidate {
            repo,
            apply,
            if_due,
        } => {
            let store = open(&cli.root)?;
            if if_due {
                match store.consolidate_if_due(&repo).map_err(|e| e.to_string())? {
                    Some(o) => writeln!(
                        out,
                        "applied: {} deprecated, {} merged, {} conflicts resolved",
                        o.deprecated, o.merged, o.conflicts_resolved
                    )
                    .ok(),
                    None => writeln!(out, "consolidation not due").ok(),
                };
            } else if apply {
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
        Cmd::Release {
            claim_id,
            session,
            by,
        } => {
            let store = open(&cli.root)?;
            if let Some(sid) = session {
                let mine: Vec<_> = store
                    .active_claims()
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|c| c.session_id == sid)
                    .collect();
                for c in &mine {
                    store.release(&c.id, &by).map_err(|e| e.to_string())?;
                }
                writeln!(out, "released {} claim(s) for session {sid}", mine.len()).ok();
            } else if let Some(id) = claim_id {
                store.release(&id, &by).map_err(|e| e.to_string())?;
                writeln!(out, "released {id}").ok();
            } else {
                return Err("provide a claim id or --session <id>".to_string());
            }
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
            if let Some(remote) = marrow_store::SharedRemote::load(&cli.root) {
                writeln!(
                    out,
                    "sharing: THIS PROJECT IS SHARED to space '{}' on {} — your mem_* tools read and write that shared brain (recall from it before answering). Other machines on the same space see your writes. File claims stay per-machine for now.",
                    remote.space, remote.url
                )
                .ok();
            }
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
                writeln!(out, "  {} — {}", m.frontmatter.id, snippet(&m.body, 220)).ok();
            }
            writeln!(out, "relevant memories ({}):", brief.relevant.len()).ok();
            for m in &brief.relevant {
                writeln!(out, "  {} — {}", m.frontmatter.id, first_line(&m.body)).ok();
            }
            if brief.suggest_ingest {
                let n = knowledge_docs(&cli.root).len();
                writeln!(
                    out,
                    "onboarding: this repo has {n} knowledge doc(s) but no memory yet — run `marrow ingest` to seed the brain so future sessions start warm."
                )
                .ok();
            }
            if brief.suggest_consolidate {
                writeln!(
                    out,
                    "maintenance: many new memories since the last cleanup — run `marrow consolidate --apply` (or mem_consolidate) to merge duplicates and retire stale notes."
                )
                .ok();
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
        Cmd::Setup { global } => setup::run(&cli.root, global, out),
        Cmd::Upgrade => setup::upgrade(out),
        Cmd::Ingest => {
            let docs = knowledge_docs(&cli.root);
            write!(out, "{}", ingest_report(&docs)).ok();
            Ok(())
        }
        Cmd::Watch { session } => {
            let store = open(&cli.root)?;
            write!(out, "{}", watch_report(&store, &session)?).ok();
            Ok(())
        }
        Cmd::Embed { provider, url } => {
            let cfg_path = cli.root.join(".marrow/.marrow.toml");
            if !cli.root.join(".marrow").exists() {
                return Err("no .marrow store here — run `marrow init` first".into());
            }
            let mut cfg = std::fs::read_to_string(&cfg_path)
                .ok()
                .and_then(|t| marrow_store::Config::from_toml(&t).ok())
                .unwrap_or_default();
            cfg.embedding.provider = provider.clone();
            if let Some(u) = url {
                cfg.embedding.url = u;
            }
            std::fs::write(&cfg_path, cfg.to_toml()).map_err(|e| e.to_string())?;
            writeln!(out, "search backend set to '{provider}'.").ok();
            let semantic = provider == "fastembed" || provider == "http";
            if semantic && !marrow_store::semantic_supported() {
                writeln!(
                    out,
                    "note: this marrow build has no semantic support, so search stays keyword-only. Reinstall with the feature:\n  cargo install --git https://github.com/aryawidjaja/marrow marrow-cli marrow-mcp --features embed-fastembed"
                )
                .ok();
            } else if semantic {
                writeln!(
                    out,
                    "semantic search active (first query downloads the model)."
                )
                .ok();
            }
            Ok(())
        }
        Cmd::Hub { cmd } => run_hub(cli.root, cmd, out),
        Cmd::Share {
            gateway,
            space,
            token,
        } => {
            let remote = marrow_store::SharedRemote {
                url: gateway.trim_end_matches('/').to_string(),
                space,
                token: token.filter(|t| !t.is_empty()),
            };
            remote.save(&cli.root).map_err(|e| e.to_string())?;
            writeln!(
                out,
                "shared: this project now routes to space '{}' on {}.",
                remote.space, remote.url
            )
            .ok();
            writeln!(
                out,
                "agents here read and write that shared brain; every other project stays local. Run `marrow unshare` to go back to local (nothing is deleted)."
            )
            .ok();
            Ok(())
        }
        Cmd::Unshare => {
            if marrow_store::SharedRemote::remove(&cli.root).map_err(|e| e.to_string())? {
                writeln!(
                    out,
                    "unshared: this project is local and private again. Your local memories are untouched."
                )
                .ok();
            } else {
                writeln!(out, "this project wasn't shared — nothing to do.").ok();
            }
            Ok(())
        }
    }
}

fn run_hub(root: PathBuf, cmd: HubCmd, out: &mut impl Write) -> Result<(), String> {
    use marrow_store::Hub;
    let mut hub = Hub::open().map_err(|e| e.to_string())?;
    match cmd {
        HubCmd::Register { name } => {
            let p = hub
                .register(&root, name.as_deref())
                .map_err(|e| e.to_string())?;
            writeln!(out, "registered '{}' -> {}", p.name, p.root.display()).ok();
        }
        HubCmd::Forget { key } => {
            if hub.forget(&key).map_err(|e| e.to_string())? {
                writeln!(out, "removed '{key}' from the hive").ok();
            } else {
                writeln!(out, "no project named or rooted at '{key}'").ok();
            }
        }
        HubCmd::List => {
            let projects = hub.projects();
            writeln!(out, "hive: {} project(s)", projects.len()).ok();
            for p in projects {
                writeln!(out, "  {} — {}", p.name, p.root.display()).ok();
            }
        }
        HubCmd::Recall { text, limit } => {
            let hits = hub.recall(&text, limit, limit);
            writeln!(out, "{} hit(s) across the hive:", hits.len()).ok();
            for h in hits {
                writeln!(
                    out,
                    "  [{}] {} — {}",
                    h.project,
                    h.memory.frontmatter.id,
                    snippet(&h.memory.body, 200)
                )
                .ok();
            }
        }
        HubCmd::Activity { limit } => {
            let events = hub.activity(limit);
            writeln!(out, "hive activity ({} event(s)):", events.len()).ok();
            for e in events {
                writeln!(
                    out,
                    "  [{}] {} — {}",
                    e.project, e.event.kind, e.event.summary
                )
                .ok();
            }
        }
        HubCmd::Remember { body, topic } => {
            let core = hub.core().map_err(|e| e.to_string())?;
            let mut memory =
                build_memory(MemoryKind::Fact, topic, None, "hub".into(), vec![], body);
            let id = core.write(&mut memory).map_err(|e| e.to_string())?;
            writeln!(out, "core memory saved: {id}").ok();
        }
    }
    Ok(())
}

fn first_line(body: &str) -> &str {
    body.trim().lines().next().unwrap_or("")
}

/// A one-glance snippet for the warm-start briefing. Memories are often dense single paragraphs, so
/// truncate by length (at a word boundary) rather than by line — the id lets the agent read the
/// full memory on demand instead of paying for every body up front.
fn snippet(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    let head = first_line(body);
    if head.chars().count() <= max_chars {
        return head.to_string();
    }
    let mut cut = head
        .char_indices()
        .map(|(i, _)| i)
        .nth(max_chars)
        .unwrap_or(head.len());
    if let Some(sp) = head[..cut].rfind(' ') {
        if sp > max_chars / 2 {
            cut = sp;
        }
    }
    format!("{}…", head[..cut].trim_end())
}

/// Shown when `marrow` is run with no subcommand — the getting-started nudge a freshly installed
/// package should give (cargo has no post-install hook, so the binary points the way itself).
fn welcome(out: &mut impl Write) -> Result<(), String> {
    writeln!(
        out,
        "Marrow {} — shared memory for AI agents.\n\n\
         Get started:\n  \
         marrow setup            wire this project into Claude Code (add --global for every project)\n  \
         marrow ingest           seed memory from an existing repo's docs\n  \
         marrow --help           all commands\n\n\
         After `marrow setup`, restart Claude Code — then sessions warm-start, avoid collisions, and\n\
         you can capture anytime with /marrow-save (or just \"save this to marrow\").\n\n\
         Search is keyword by default; for smarter meaning-based recall, enable semantic search\n\
         (opt-in, needs an embedding model): see `marrow embed` and the README.\n\n\
         Docs: https://github.com/aryawidjaja/marrow",
        env!("CARGO_PKG_VERSION"),
    )
    .ok();
    Ok(())
}

/// The onboarding instruction an agent acts on: the project's knowledge docs plus a directive to
/// distill (not dump) them into memory. Marrow does no LLM work here — the agent reads the docs and
/// writes the distilled memories itself.
fn ingest_report(docs: &[(String, u64)]) -> String {
    if docs.is_empty() {
        return "No knowledge docs (Markdown) found under this project.\n".to_string();
    }
    let total: u64 = docs.iter().map(|(_, n)| n).sum();
    let mut s = format!(
        "Found {} knowledge doc(s) (~{}). To seed Marrow's memory, read each and save the durable \
         decisions, facts, and architecture with mem_write — distill, don't paste whole files. Call \
         mem_recall first and skip anything already saved.\n\nFiles:\n",
        docs.len(),
        human_bytes(total),
    );
    for (path, size) in docs {
        s.push_str(&format!("  {path} ({})\n", human_bytes(*size)));
    }
    s
}

fn watch_report(store: &Store, session: &str) -> Result<String, String> {
    use std::collections::HashSet;
    let wm_path = store.root().join(".marrow/.watch").join(session);
    let watermark: u64 = std::fs::read_to_string(&wm_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    let events = store.activity(500).map_err(|e| e.to_string())?;
    let latest = events.first().map(|e| e.seq).unwrap_or(watermark);

    let short = |s: &str| s.chars().take(6).collect::<String>();
    let mut lines = Vec::new();
    for e in events.iter().rev() {
        if e.seq <= watermark || !(e.kind == "claim" || e.kind == "progress") {
            continue;
        }
        if let Some(sid) = e.data.get("session_id").and_then(|v| v.as_str()) {
            if sid != session {
                lines.push(format!("  {} (session {})", e.summary, short(sid)));
            }
        }
    }

    let others: HashSet<String> = store
        .active_claims()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|c| c.session_id != session)
        .map(|c| c.session_id)
        .collect();

    // Bound a single injection: in a busy hive many events can land between two prompts, and this
    // text becomes permanent conversation history. Keep the most recent few; count the rest.
    const MAX_LINES: usize = 8;
    let overflow = lines.len().saturating_sub(MAX_LINES);
    if overflow > 0 {
        lines = lines.split_off(overflow);
        lines.insert(
            0,
            format!("  (+{overflow} earlier update(s) from other sessions)"),
        );
    }

    let mut out = String::new();
    if !lines.is_empty() || !others.is_empty() {
        out.push_str("Marrow — what other sessions are doing:\n");
        out.push_str(&lines.join("\n"));
        if !lines.is_empty() {
            out.push('\n');
        }
        if !others.is_empty() {
            out.push_str(&format!(
                "  ({} other session(s) hold active claims — don't edit their files in parallel; offer to help if they look stuck.)\n",
                others.len()
            ));
        }
    }

    if latest > watermark {
        if let Some(dir) = wm_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&wm_path, latest.to_string());
    }
    Ok(out)
}

fn human_bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} B")
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

#[cfg(test)]
mod brief_tests {
    use super::snippet;

    #[test]
    fn snippet_caps_dense_paragraphs_at_a_word_boundary() {
        let dense = "This is a single dense paragraph with no newlines that a decision body \
                     often is, and it keeps going well past the cap so it must be truncated.";
        let s = snippet(dense, 40);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= 41);
        assert!(!s.contains("truncated"));
    }

    #[test]
    fn snippet_leaves_short_bodies_intact() {
        assert_eq!(snippet("short body", 220), "short body");
    }
}
