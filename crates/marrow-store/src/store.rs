//! The memory store: markdown files as the source of truth, SQLite as a derived index.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use marrow_episodic::{EpisodicLog, Event, NewEvent};
use marrow_memdocs::{to_markdown, validate, CodeAnchor, Memory, Ref, RefKind, Violation};
use rusqlite::Connection;
use ulid::Ulid;

use crate::config::Config;
use crate::convert::{kind_str, status_str};
use crate::embed::{cosine, rrf, Embedder, HashEmbedder};
use crate::index::{self, IndexRow};
use crate::query::{estimate_tokens, Query};
use crate::staleness::{check_memory, StaleHit};
use crate::{integrity, util};

/// Errors returned by store operations.
#[derive(Debug)]
pub enum Error {
    /// The memory failed schema validation; carries every violation.
    Invalid(Vec<Violation>),
    /// A uniqueness or lifecycle constraint was violated.
    Conflict(String),
    /// Signing is enabled but no key is available.
    Unsigned,
    /// The requested memory does not exist.
    NotFound(String),
    /// A document on disk could not be parsed.
    Parse(String),
    /// Filesystem error.
    Io(String),
    /// Index/database error.
    Db(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(vs) => {
                let joined = vs
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "invalid memory: {joined}")
            }
            Error::Conflict(m) => write!(f, "conflict: {m}"),
            Error::Unsigned => write!(f, "signing enabled but no MARROW_HMAC_KEY available"),
            Error::NotFound(id) => write!(f, "memory not found: {id}"),
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::Io(m) => write!(f, "io error: {m}"),
            Error::Db(m) => write!(f, "index error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Db(e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// A Marrow memory store rooted at a project directory.
pub struct Store {
    root: PathBuf,
    conn: Connection,
    pub(crate) config: Config,
    key: Option<Vec<u8>>,
    pub(crate) embedder: Option<Box<dyn Embedder>>,
    episodic: RefCell<EpisodicLog>,
    pub(crate) distiller: Box<dyn crate::consolidate::Distiller>,
}

/// Resolve the effective store root: if `start` has no `.marrow`, walk up to the nearest ancestor
/// that does, so launching from a subdirectory still opens the project's brain instead of a new
/// empty one. Falls back to `start` when no ancestor store exists.
fn resolve_root(start: &Path) -> PathBuf {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    if start.join(".marrow").is_dir() {
        return start;
    }
    let mut cur = start.as_path();
    while let Some(parent) = cur.parent() {
        if parent.join(".marrow").is_dir() {
            return parent.to_path_buf();
        }
        cur = parent;
    }
    start
}

impl Store {
    /// Initialize a new store under `root/.marrow`, creating the layout and config.
    pub fn init(root: impl AsRef<Path>) -> Result<Store, Error> {
        let root = root.as_ref().to_path_buf();
        let marrow = root.join(".marrow");
        fs::create_dir_all(marrow.join("memory"))?;
        fs::create_dir_all(marrow.join(".index"))?;
        let config = Config::default();
        if !marrow.join(".marrow.toml").exists() {
            fs::write(marrow.join(".marrow.toml"), config.to_toml())?;
        }
        Self::open(root)
    }

    /// Open an existing store (initializing the index schema if needed).
    pub fn open(root: impl AsRef<Path>) -> Result<Store, Error> {
        let root = resolve_root(root.as_ref());
        let marrow = root.join(".marrow");
        let config = match fs::read_to_string(marrow.join(".marrow.toml")) {
            Ok(t) => Config::from_toml(&t).map_err(|e| Error::Parse(e.to_string()))?,
            Err(_) => Config::default(),
        };
        fs::create_dir_all(marrow.join(".index"))?;
        let conn = Connection::open(marrow.join(".index/sqlite.db"))?;
        // Another writer (a second device on the served backbone, say) may hold the lock; wait for
        // it rather than failing the open.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        index::init_schema(&conn)?;
        let key = if config.sign {
            std::env::var("MARROW_HMAC_KEY")
                .ok()
                .map(|s| s.into_bytes())
        } else {
            None
        };
        let embedder = build_embedder(&config);
        let distiller = crate::consolidate::build_distiller(&config);
        let episodic = EpisodicLog::open(&root).map_err(|e| Error::Io(e.to_string()))?;
        Ok(Store {
            root,
            conn,
            config,
            key,
            embedder,
            episodic: RefCell::new(episodic),
            distiller,
        })
    }

    /// Append an event to the episodic / audit ledger.
    fn record(&self, ev: NewEvent) -> Result<(), Error> {
        self.episodic
            .borrow_mut()
            .append(ev)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(())
    }

    /// Record an arbitrary agent-authored episodic event (e.g. an observation or correction).
    pub fn log_event(&self, kind: &str, actor: &str, summary: &str) -> Result<(), Error> {
        self.record(NewEvent::new(kind, actor, summary))
    }

    /// Record a retrieval: which memory ids were recalled for a query. This is what makes an
    /// answer traceable back to the knowledge that produced it, and it is also what teaches the
    /// brain which memories are worth reaching for — see [`Store::recall_counts`].
    pub fn log_retrieval(&self, actor: &str, query: &str, ids: &[String]) -> Result<(), Error> {
        let mut ev = NewEvent::new(
            "retrieve",
            actor,
            &format!(
                "recalled '{query}' → {} memor{}",
                ids.len(),
                if ids.len() == 1 { "y" } else { "ies" }
            ),
        );
        ev.data = serde_json::json!({ "query": query, "ids": ids });
        index::bump_recalls(&self.conn, ids).map_err(|e| Error::Db(e.to_string()))?;
        self.record(ev)
    }

    /// How many times each memory has been recalled. A memory that keeps proving useful is easier
    /// to recall again; one nobody ever reaches for stays where it is.
    pub fn recall_counts(&self) -> Result<HashMap<String, u32>, Error> {
        index::recall_counts(&self.conn)
            .map(|v| v.into_iter().collect())
            .map_err(|e| Error::Db(e.to_string()))
    }

    /// Rebuild the recall counts from the episodic ledger. Every retrieval Marrow has ever served
    /// is already recorded there, so a store that predates the counts still knows its own history.
    pub fn rebuild_recall_counts(&self) -> Result<usize, Error> {
        let mut counts: HashMap<String, u32> = HashMap::new();
        for ev in self.history()? {
            if ev.kind != "retrieve" {
                continue;
            }
            for id in ev.data["ids"].as_array().into_iter().flatten() {
                if let Some(id) = id.as_str() {
                    *counts.entry(id.to_string()).or_default() += 1;
                }
            }
        }
        let flat: Vec<(String, u32)> = counts.into_iter().collect();
        index::reset_recalls(&self.conn, &flat).map_err(|e| Error::Db(e.to_string()))?;
        Ok(flat.len())
    }

    /// Append an event carrying a structured payload. Used by the coordination plane
    /// (claims, activity) so events round-trip through the same tamper-evident ledger.
    pub(crate) fn log_data(
        &self,
        kind: &str,
        actor: &str,
        summary: &str,
        data: serde_json::Value,
    ) -> Result<(), Error> {
        let mut ev = NewEvent::new(kind, actor, summary);
        ev.data = data;
        self.record(ev)
    }

    /// The full episodic / audit history, oldest first.
    pub fn history(&self) -> Result<Vec<Event>, Error> {
        self.episodic
            .borrow()
            .read_all()
            .map_err(|e| Error::Io(e.to_string()))
    }

    /// Verify the audit chain. `Ok(())` if intact, else the `seq` of the first broken entry.
    pub fn verify_log(&self) -> Result<(), u64> {
        self.episodic.borrow().verify()
    }

    /// Override the signing key (mainly for tests and embedding).
    pub fn set_key(&mut self, key: impl Into<Vec<u8>>) {
        self.key = Some(key.into());
    }

    /// Inject an embedder (used by tests and embedding integrations).
    pub fn set_embedder(&mut self, embedder: Box<dyn Embedder>) {
        self.embedder = Some(embedder);
    }

    /// Inject a distiller (used by tests and to plug in an LLM-backed consolidator).
    pub fn set_distiller(&mut self, distiller: Box<dyn crate::consolidate::Distiller>) {
        self.distiller = distiller;
    }

    /// The project root this store is anchored to.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The configured embedding backend ("none"/"hash" = keyword search).
    pub fn embedding_provider(&self) -> &str {
        &self.config.embedding.provider
    }

    fn memory_dir(&self) -> PathBuf {
        self.root.join(".marrow/memory")
    }

    /// Relative path (within `memory/`) for a memory: `<kind>/<id>.md`.
    fn rel_path(&self, memory: &Memory) -> String {
        format!(
            "{}/{}.md",
            kind_str(memory.frontmatter.kind),
            memory.frontmatter.id
        )
    }

    /// A memory's file lives under its kind, so changing the kind moves the file. Drop the copy at
    /// the old path, or the id would exist twice on disk and a later reindex could pick either.
    fn drop_stale_file(&self, id: &str, rel: &str) {
        let Ok(Some(old)) = index::path_of(&self.conn, id) else {
            return;
        };
        if old != rel {
            let _ = fs::remove_file(self.memory_dir().join(old));
        }
    }

    /// Validate, sign, persist atomically, and index a memory. Returns its id.
    pub fn write(&self, memory: &mut Memory) -> Result<String, Error> {
        let now = util::now_rfc3339();
        {
            let fm = &mut memory.frontmatter;
            if fm.id.trim().is_empty() {
                fm.id = Ulid::new().to_string();
            }
            if fm.created_at.trim().is_empty() {
                fm.created_at = now.clone();
            }
            fm.updated_at = now;
            if fm.scope.project_id.trim().is_empty() {
                fm.scope.project_id = self.config.project_id.clone();
            }
        }

        validate(memory).map_err(Error::Invalid)?;
        self.enforce_unique_active_decision(memory)?;

        if self.config.sign {
            let key = self.key.as_ref().ok_or(Error::Unsigned)?;
            memory.frontmatter.hmac = Some(integrity::sign(memory, key));
        }

        let rel = self.rel_path(memory);
        let abs = self.memory_dir().join(&rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&abs, &to_markdown(memory))?;
        self.drop_stale_file(&memory.frontmatter.id, &rel);

        index::upsert(&self.conn, &self.row_of(memory, &rel))?;
        self.embed_memory(memory)?;

        let fm = &memory.frontmatter;
        let summary = format!(
            "wrote {} '{}'",
            kind_str(fm.kind),
            fm.topic.as_deref().unwrap_or(&fm.id)
        );
        self.record(NewEvent::new("write", &fm.provenance.written_by, &summary).memory(&fm.id))?;
        Ok(memory.frontmatter.id.clone())
    }

    /// Write a memory anchored to a code symbol: seeds a structural fingerprint of the symbol
    /// so the memory can be staleness-checked, then writes it. Shared by the CLI, MCP, and web.
    pub fn write_anchored(
        &self,
        repo_root: &Path,
        file: &str,
        symbol: &str,
        memory: &mut Memory,
    ) -> Result<String, Error> {
        let core = marrow_core::seed_anchor(repo_root, file, symbol)
            .ok_or_else(|| Error::NotFound(format!("symbol {symbol} in {file}")))?;
        memory.frontmatter.refs.push(Ref {
            kind: RefKind::Symbol,
            value: format!("{file}::{symbol}"),
        });
        memory.frontmatter.code_anchors.push(CodeAnchor {
            file_path: core.file_path,
            symbol: core.symbol,
            snippet: core.snippet,
            fingerprint: core.fingerprint,
            norm: core.norm,
        });
        self.write(memory)
    }

    /// Edit a memory in place, keeping its id and lineage: update topic, body, and/or tags, then
    /// rewrite the markdown, re-index, and re-embed. Returns false if there is no such memory.
    pub fn update(
        &self,
        id: &str,
        topic: Option<String>,
        body: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<bool, Error> {
        let Some(mut memory) = self.read(id)? else {
            return Ok(false);
        };
        if let Some(t) = topic {
            memory.frontmatter.topic = (!t.trim().is_empty()).then_some(t);
        }
        if let Some(b) = body {
            memory.body = b;
        }
        if let Some(tg) = tags {
            memory.frontmatter.tags = tg;
        }
        memory.frontmatter.updated_at = util::now_rfc3339();
        validate(&memory).map_err(Error::Invalid)?;
        if self.config.sign {
            let key = self.key.as_ref().ok_or(Error::Unsigned)?;
            memory.frontmatter.hmac = Some(integrity::sign(&memory, key));
        }
        let rel = self.rel_path(&memory);
        atomic_write(&self.memory_dir().join(&rel), &to_markdown(&memory))?;
        self.drop_stale_file(id, &rel);
        index::upsert(&self.conn, &self.row_of(&memory, &rel))?;
        self.embed_memory(&memory)?;
        let summary = format!(
            "edited '{}'",
            memory.frontmatter.topic.as_deref().unwrap_or(id)
        );
        self.record(NewEvent::new("edit", "web", &summary).memory(id))?;
        Ok(true)
    }

    /// Delete a memory outright: remove its markdown file, index row, and embedding. Returns false
    /// if there is no such memory.
    pub fn delete(&self, id: &str) -> Result<bool, Error> {
        let Some(rel) = index::path_of(&self.conn, id)? else {
            return Ok(false);
        };
        let _ = fs::remove_file(self.memory_dir().join(rel));
        index::delete(&self.conn, id)?;
        self.record(NewEvent::new("delete", "web", &format!("deleted {id}")).memory(id))?;
        Ok(true)
    }

    /// Embed a memory's topic+body and store the vector, if an embedder is configured.
    fn embed_memory(&self, memory: &Memory) -> Result<(), Error> {
        if let Some(embedder) = &self.embedder {
            let text = embed_text(memory);
            if let Ok(vec) = embedder.embed_one(&text) {
                index::upsert_vector(&self.conn, &memory.frontmatter.id, &vec)?;
            }
        }
        Ok(())
    }

    /// At most one active decision may exist per (topic, project).
    fn enforce_unique_active_decision(&self, memory: &Memory) -> Result<(), Error> {
        use marrow_memdocs::{MemoryKind, Status};
        let fm = &memory.frontmatter;
        if fm.kind != MemoryKind::Decision || fm.status != Status::Active {
            return Ok(());
        }
        let Some(topic) = &fm.topic else {
            return Ok(());
        };
        let q = Query {
            kind: Some(MemoryKind::Decision),
            status: Some(Status::Active),
            topic: Some(topic.clone()),
            project_id: Some(fm.scope.project_id.clone()),
            ..Query::default()
        };
        let clash = index::query(&self.conn, &q, &util::now_rfc3339())?
            .into_iter()
            .any(|r| r.id != fm.id);
        if clash {
            return Err(Error::Conflict(format!(
                "an active decision already exists for topic '{topic}' in project '{}'",
                fm.scope.project_id
            )));
        }
        Ok(())
    }

    /// Read a memory by id, if present.
    pub fn read(&self, id: &str) -> Result<Option<Memory>, Error> {
        let Some(rel) = index::path_of(&self.conn, id)? else {
            return Ok(None);
        };
        let text = fs::read_to_string(self.memory_dir().join(rel))?;
        let memory = marrow_memdocs::parse(&text).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(Some(memory))
    }

    /// Lightweight listing of all indexed memories (most recent first).
    pub fn list(&self) -> Result<Vec<IndexRow>, Error> {
        Ok(index::query(
            &self.conn,
            &Query::default(),
            &util::now_rfc3339(),
        )?)
    }

    /// A cheap fingerprint of the store's current state: enough to tell whether anything changed,
    /// without reading a single memory body. The dashboard polls this, so it must stay trivial.
    pub fn revision(&self) -> Result<String, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*), COALESCE(MAX(updated_at), '') FROM memories WHERE status = 'active'",
        )?;
        let (count, latest): (i64, String) = stmt.query_row([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(format!("{count}:{latest}"))
    }

    /// The project's feature areas with their memory counts, busiest first — the table of contents
    /// for this brain. An agent reads this before writing so it files into an area that already
    /// exists instead of inventing a near-duplicate ("auth" vs "authentication"). Memories with no
    /// area are reported under an empty name; they stay fully searchable, they're just unfiled.
    pub fn areas(&self) -> Result<Vec<(String, usize)>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT area, COUNT(*) FROM memories WHERE status = 'active' GROUP BY area")?;
        let mut areas: Vec<(String, usize)> = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
            })?
            .collect::<Result<_, _>>()?;
        areas.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(areas)
    }

    /// Structured query, loading full memories under an optional token budget.
    pub fn query(&self, q: &Query) -> Result<Vec<Memory>, Error> {
        let rows = index::query(&self.conn, q, &util::now_rfc3339())?;
        self.load_budgeted(rows, q.max_tokens)
    }

    /// The stored embedding vector for every active memory, so meaning can be compared across
    /// stores (e.g. cross-project links in the hive). Empty when the store has no embeddings.
    pub fn vectors(&self) -> Result<Vec<(String, Vec<f32>)>, Error> {
        let ids: Vec<String> = self
            .list()?
            .into_iter()
            .filter(|r| r.status == "active")
            .map(|r| r.id)
            .collect();
        Ok(index::vectors_for(&self.conn, &ids)?)
    }

    /// Semantic neighbours: for each active memory, up to `top_k` others whose embedding cosine is
    /// at least `min_sim`. Each pair appears once. Empty when the store has no real (non-hash)
    /// embeddings — meaning-based links need a semantic backend.
    pub fn related(&self, top_k: usize, min_sim: f32) -> Result<Vec<(String, String, f32)>, Error> {
        let ids: Vec<String> = self
            .list()?
            .into_iter()
            .filter(|r| r.status == "active")
            .map(|r| r.id)
            .collect();
        let vecs = index::vectors_for(&self.conn, &ids)?;
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (i, (_, va)) in vecs.iter().enumerate() {
            let mut sims: Vec<(usize, f32)> = vecs
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(j, (_, vb))| (j, crate::embed::cosine(va, vb)))
                .filter(|(_, s)| *s >= min_sim)
                .collect();
            sims.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (j, s) in sims.into_iter().take(top_k) {
                let (lo, hi) = if i < j { (i, j) } else { (j, i) };
                if seen.insert((lo, hi)) {
                    edges.push((vecs[lo].0.clone(), vecs[hi].0.clone(), s));
                }
            }
        }
        Ok(edges)
    }

    /// Hybrid search: keyword (FTS5/BM25) fused with semantic (vector cosine) via weighted
    /// RRF. `hybrid_weight` of 0 (or no embedder) is exactly keyword search.
    pub fn search(&self, text: &str, q: &Query) -> Result<Vec<Memory>, Error> {
        let now = util::now_rfc3339();
        let keyword: Vec<String> = index::search(&self.conn, text, q, &now)?
            .into_iter()
            .map(|r| r.id)
            .collect();

        let w = q
            .hybrid_weight
            .unwrap_or(self.config.embedding.default_weight);
        let semantic = match (&self.embedder, w > 0.0) {
            (Some(embedder), true) => self.semantic_ranking(text, q, embedder.as_ref(), &now)?,
            _ => Vec::new(),
        };

        let fused = rrf(&keyword, &semantic, w);
        let rows: Vec<IndexRow> = fused
            .into_iter()
            .filter_map(|id| index::query_one(&self.conn, &id).ok().flatten())
            .collect();
        self.load_budgeted(rows, q.max_tokens)
    }

    /// Rank the filtered candidate set by cosine similarity to the query embedding.
    fn semantic_ranking(
        &self,
        text: &str,
        q: &Query,
        embedder: &dyn Embedder,
        now: &str,
    ) -> Result<Vec<String>, Error> {
        let candidates: Vec<String> = index::query(&self.conn, q, now)?
            .into_iter()
            .map(|r| r.id)
            .collect();
        let qvec = embedder
            .embed_one(text)
            .map_err(|e| Error::Db(e.to_string()))?;
        let mut scored: Vec<(String, f32)> = index::vectors_for(&self.conn, &candidates)?
            .into_iter()
            .map(|(id, v)| (id, cosine(&qvec, &v)))
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(SEMANTIC_TOP_K);
        Ok(scored.into_iter().map(|(id, _)| id).collect())
    }

    fn load_budgeted(
        &self,
        rows: Vec<IndexRow>,
        max_tokens: Option<usize>,
    ) -> Result<Vec<Memory>, Error> {
        let mut out = Vec::new();
        let mut used = 0usize;
        for row in rows {
            let Some(memory) = self.read(&row.id)? else {
                continue;
            };
            let cost = estimate_tokens(&memory.body) + estimate_tokens(&row.topic);
            if let Some(budget) = max_tokens {
                if !out.is_empty() && used + cost > budget {
                    break;
                }
            }
            used += cost;
            out.push(memory);
        }
        Ok(out)
    }

    /// Mark `old_id` superseded and write `new` in its place, recording the lineage.
    pub fn supersede(&self, old_id: &str, new: &mut Memory) -> Result<String, Error> {
        use marrow_memdocs::Status;
        let mut old = self
            .read(old_id)?
            .ok_or_else(|| Error::NotFound(old_id.to_string()))?;
        old.frontmatter.status = Status::Superseded;
        // A revision stays in the same part of the brain unless it explicitly says otherwise, so a
        // superseding memory inherits the old one's area rather than silently becoming unfiled.
        if new.frontmatter.area.is_none() {
            new.frontmatter.area = old.frontmatter.area.clone();
        }
        self.write(&mut old)?;
        if !new.frontmatter.supersedes.iter().any(|s| s == old_id) {
            new.frontmatter.supersedes.push(old_id.to_string());
        }
        let actor = new.frontmatter.provenance.written_by.clone();
        let new_id = self.write(new)?;
        self.record(
            NewEvent::new(
                "supersede",
                &actor,
                &format!("superseded {old_id} with {new_id}"),
            )
            .memory(&new_id),
        )?;
        Ok(new_id)
    }

    /// Every code anchor across all memories that no longer matches the live repo.
    pub fn list_stale(&self, repo_root: &Path) -> Result<Vec<StaleHit>, Error> {
        let mut hits = Vec::new();
        for row in self.list()? {
            if let Some(memory) = self.read(&row.id)? {
                hits.extend(check_memory(repo_root, &memory));
            }
        }
        Ok(hits)
    }

    /// Verify a memory's HMAC. `None` if no signing key is configured.
    pub fn verify(&self, memory: &Memory) -> Option<bool> {
        self.key.as_ref().map(|k| integrity::verify(memory, k))
    }

    /// Rebuild the index from the markdown files on disk. Returns the count indexed.
    pub fn reindex(&self) -> Result<usize, Error> {
        // Recreate the FTS table from scratch so `doctor` also picks up schema/tokenizer
        // changes (e.g. switching on the porter stemmer) on stores built by older versions.
        self.conn
            .execute_batch("DROP TABLE IF EXISTS memories_fts;")
            .map_err(|e| Error::Db(e.to_string()))?;
        index::init_schema(&self.conn)?;
        index::clear(&self.conn)?;
        let mut count = 0;
        for path in markdown_files(&self.memory_dir()) {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(memory) = marrow_memdocs::parse(&text) else {
                continue;
            };
            let rel = path
                .strip_prefix(self.memory_dir())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            index::upsert(&self.conn, &self.row_of(&memory, &rel))?;
            self.embed_memory(&memory)?;
            count += 1;
        }
        // `clear` wiped the counts with the rest of the derived index; the ledger still has them.
        self.rebuild_recall_counts()?;
        Ok(count)
    }

    /// Recompute and store embeddings for every memory. Returns the count embedded.
    pub fn reembed(&self) -> Result<usize, Error> {
        if self.embedder.is_none() {
            return Ok(0);
        }
        let mut n = 0;
        for row in self.list()? {
            if let Some(memory) = self.read(&row.id)? {
                self.embed_memory(&memory)?;
                n += 1;
            }
        }
        Ok(n)
    }

    fn row_of(&self, memory: &Memory, rel: &str) -> IndexRow {
        let fm = &memory.frontmatter;
        let tags = if fm.tags.is_empty() {
            String::new()
        } else {
            format!(",{},", fm.tags.join(","))
        };
        let expires_at = fm
            .decay
            .as_ref()
            .and_then(|d| d.expires_at.clone())
            .unwrap_or_default();
        IndexRow {
            id: fm.id.clone(),
            kind: kind_str(fm.kind).into(),
            status: status_str(fm.status).into(),
            topic: fm.topic.clone().unwrap_or_default(),
            area: fm.area.clone().unwrap_or_default(),
            project_id: fm.scope.project_id.clone(),
            written_by: fm.provenance.written_by.clone(),
            model: fm.provenance.model.clone().unwrap_or_default(),
            confidence: fm.confidence,
            created_at: fm.created_at.clone(),
            updated_at: fm.updated_at.clone(),
            expires_at,
            tags,
            path: rel.to_string(),
            body: memory.body.clone(),
        }
    }
}

/// Write `text` to `path` atomically (temp file in the same dir, then rename).
/// Upper bound on semantic candidates fed into fusion, so a `w=1` search never returns the
/// whole store.
const SEMANTIC_TOP_K: usize = 64;

/// The text embedded for a memory: its topic and body.
fn embed_text(memory: &Memory) -> String {
    format!(
        "{}\n{}",
        memory.frontmatter.topic.clone().unwrap_or_default(),
        memory.body
    )
}

/// Construct the embedder named by config, if its backend is available.
fn build_embedder(config: &Config) -> Option<Box<dyn Embedder>> {
    match config.embedding.provider.as_str() {
        "hash" => Some(Box::new(HashEmbedder::new(config.embedding.dim))),
        #[cfg(feature = "embed-http")]
        "http" => Some(Box::new(crate::embed_http::HttpEmbedder::from_config(
            &config.embedding,
        ))),
        #[cfg(feature = "embed-fastembed")]
        "fastembed" => crate::embed_fastembed::FastEmbedder::from_config(&config.embedding)
            .map(|e| Box::new(e) as Box<dyn Embedder>),
        _ => None,
    }
}

fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("memory")
    ));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// Recursively collect `*.md` files under `dir`.
fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "md") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}
