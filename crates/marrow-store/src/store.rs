//! The memory store: markdown files as the source of truth, SQLite as a derived index.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use marrow_episodic::{EpisodicLog, Event, NewEvent};
use marrow_memdocs::{to_markdown, validate, CodeAnchor, Memory, Ref, RefKind, Status, Violation};
use rusqlite::Connection;
use ulid::Ulid;

use crate::associative::MAX_HOPS;
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

    /// Whether semantic retrieval is actually usable in this process. A config can request
    /// `fastembed`/`http` while the binary was built without that feature; reporting only the
    /// configured name would falsely claim semantic search is active.
    pub fn semantic_active(&self) -> bool {
        self.embedder.is_some()
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
        if let Some(existing_id) = self.enforce_unique_active_topic(memory)? {
            memory.frontmatter.id = existing_id.clone();
            return Ok(existing_id);
        }

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
        Self::attach_anchor(memory, core);
        self.write(memory)
    }

    /// Anchor every resolvable `file::symbol` reference named by a memory, then write it. Invalid
    /// passing mentions are ignored; exact code references that resolve become independent stale
    /// detectors instead of silently anchoring only the first symbol.
    pub fn write_auto_anchored(
        &self,
        repo_root: &Path,
        refs: &[(String, String)],
        memory: &mut Memory,
    ) -> Result<String, Error> {
        for (file, symbol) in refs {
            if let Some(core) = marrow_core::seed_anchor(repo_root, file, symbol) {
                Self::attach_anchor(memory, core);
            }
        }
        self.write(memory)
    }

    fn attach_anchor(memory: &mut Memory, core: marrow_core::Anchor) {
        let joined = format!("{}::{}", core.file_path, core.symbol);
        if !memory
            .frontmatter
            .refs
            .iter()
            .any(|reference| reference.kind == RefKind::Symbol && reference.value == joined)
        {
            memory.frontmatter.refs.push(Ref {
                kind: RefKind::Symbol,
                value: joined,
            });
        }
        if memory
            .frontmatter
            .code_anchors
            .iter()
            .any(|anchor| anchor.file_path == core.file_path && anchor.symbol == core.symbol)
        {
            return;
        }
        memory.frontmatter.code_anchors.push(CodeAnchor {
            file_path: core.file_path,
            symbol: core.symbol,
            snippet: core.snippet,
            fingerprint: core.fingerprint,
            norm: core.norm,
        });
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

    /// Active decisions are versioned nodes: at most one per (topic, project). Facts/entities may
    /// share a topic as a deliberate cluster, but an exact repeated write of any kind is
    /// idempotent rather than growing the store.
    ///
    /// Returns the existing id for an exact retry.
    fn enforce_unique_active_topic(&self, memory: &Memory) -> Result<Option<String>, Error> {
        use marrow_memdocs::Status;
        let fm = &memory.frontmatter;
        if fm.status != Status::Active {
            return Ok(None);
        }
        let Some(topic) = &fm.topic else {
            return Ok(None);
        };
        let q = Query {
            kind: Some(fm.kind),
            status: Some(Status::Active),
            topic: Some(topic.clone()),
            project_id: Some(fm.scope.project_id.clone()),
            ..Query::default()
        };
        let clash = index::query(&self.conn, &q, &util::now_rfc3339())?
            .into_iter()
            .find(|row| row.id != fm.id);
        let Some(existing) = clash else {
            return Ok(None);
        };
        let same_kind = existing.kind == kind_str(fm.kind);
        let normalize = |body: &str| body.split_whitespace().collect::<Vec<_>>().join(" ");
        if same_kind && normalize(&existing.body) == normalize(&memory.body) {
            return Ok(Some(existing.id));
        }
        if fm.kind != marrow_memdocs::MemoryKind::Decision {
            return Ok(None);
        }
        Err(Error::Conflict(format!(
            "active decision {} already owns topic '{topic}' in project '{}'; use mem_supersede(id=\"{}\", ...) to update it, or choose a more precise topic",
            existing.id, fm.scope.project_id, existing.id
        )))
    }

    /// Read a memory by id, if present. Served entirely from the index row: the row carries the full
    /// serialized document, so the exported markdown file is never read. The row is the source of
    /// truth for reads, so an out-of-band edit to a markdown file on disk is invisible to `read`
    /// until a `reindex` rebuilds the rows from disk.
    pub fn read(&self, id: &str) -> Result<Option<Memory>, Error> {
        let Some(row) = index::query_one(&self.conn, id)? else {
            return Ok(None);
        };
        Ok(Some(self.hydrate(&row)?))
    }

    /// Reconstruct a [`Memory`] from an index row by parsing its `doc` column, with no disk access.
    /// Rows with an empty `doc` fall back to the exported markdown file (see `IndexRow::doc`).
    fn hydrate(&self, row: &IndexRow) -> Result<Memory, Error> {
        let text = if row.doc.is_empty() {
            fs::read_to_string(self.memory_dir().join(&row.path))?
        } else {
            row.doc.clone()
        };
        marrow_memdocs::parse(&text).map_err(|e| Error::Parse(e.to_string()))
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

    /// Keyword-lane retrieval candidates for `text`, capped at `limit`: the FTS/BM25 hits, topic-tier
    /// reranked, exactly as [`Store::search`]'s keyword lane produces them — but returned raw, before
    /// any semantic fusion, as a primitive for the backend port.
    pub(crate) fn keyword_candidates(
        &self,
        text: &str,
        limit: usize,
    ) -> Result<Vec<IndexRow>, Error> {
        let now = util::now_rfc3339();
        let candidates = index::search(&self.conn, text, &Query::default(), &now)?;
        let mut ranked = crate::rank::rerank(candidates, text);
        ranked.truncate(limit);
        Ok(ranked)
    }

    /// The `top_k` active memories nearest `seed` by embedding cosine, as `(id, score)`. Empty when
    /// the store has no embeddings. Brute-force over the stored vectors — the local counterpart to a
    /// Postgres pgvector ANN search.
    pub(crate) fn vector_candidates(
        &self,
        seed: &[f32],
        top_k: usize,
        filter: &Query,
    ) -> Result<Vec<(String, f32)>, Error> {
        // Push the structured filter INTO the search: restrict the candidate rows FIRST, then take the
        // top-k. A Postgres backend does this as a filtered pgvector ANN; locally it is a filter over
        // the active-vector scan. Post-filtering a global top-k would drop matching-but-lower-cosine
        // rows and is exactly the regression this avoids.
        let allowed = self.filter_ids(filter)?;
        let mut scored: Vec<(String, f32)> = self
            .vectors()?
            .into_iter()
            .filter(|(id, _)| allowed.as_ref().is_none_or(|set| set.contains(id)))
            .map(|(id, v)| (id, cosine(seed, &v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        Ok(scored)
    }

    /// The id allow-set a structured `filter` selects, or `None` when the filter is unconstrained (no
    /// restriction to intersect). Used to scope the vector lane without loading any embeddings.
    fn filter_ids(
        &self,
        filter: &Query,
    ) -> Result<Option<std::collections::HashSet<String>>, Error> {
        let unconstrained = filter.kind.is_none()
            && filter.status.is_none()
            && filter.topic.is_none()
            && filter.project_id.is_none()
            && filter.min_confidence.is_none()
            && filter.tag.is_none()
            && !filter.exclude_expired;
        if unconstrained {
            return Ok(None);
        }
        let now = util::now_rfc3339();
        Ok(Some(
            index::query(&self.conn, filter, &now)?
                .into_iter()
                .map(|r| r.id)
                .collect(),
        ))
    }

    /// Assemble the bounded recall corpus: the keyword and vector candidates for the query, plus the
    /// edge-neighbourhood reachable from them within [`MAX_HOPS`], and their vectors. This is the
    /// whole-store `list()`+`vectors()` load's replacement — every row it pulls is a bounded index
    /// lookup, so the work is `O(candidates + their neighbourhood)`, never `O(store)`. An
    /// [`EdgeCorpus`] built over the result carries the same edges the full scan would for every node
    /// activation actually reaches, so spreading over it matches the full-scan recall.
    pub(crate) fn bounded_corpus(
        &self,
        seeds: &[Memory],
        text: &str,
        candidates: usize,
    ) -> Result<BoundedCorpus, Error> {
        let half = candidates.div_ceil(2);
        // One loader per recall: the active vector set is read once, and every topic/tag star is
        // memoised, so a dense store costs one vector load plus one query per DISTINCT topic/tag —
        // never one per node.
        let mut neigh = Neighborhood::new(self)?;
        let mut selected: std::collections::BTreeMap<String, IndexRow> =
            std::collections::BTreeMap::new();

        // Base lane 1 — keyword (FTS/BM25).
        neigh.stats.keyword_candidate_calls += 1;
        for row in self.keyword_candidates(text, half)? {
            selected.entry(row.id.clone()).or_insert(row);
        }
        // Base lane 2 — vector (top-k over the once-loaded active set; the pgvector-ANN seam).
        if let Some(embedder) = &self.embedder {
            if let Ok(qvec) = embedder.embed_one(text) {
                for id in neigh.query_vector_candidates(&qvec, half) {
                    if let std::collections::btree_map::Entry::Vacant(entry) = selected.entry(id) {
                        if let Some(row) = index::query_one(&self.conn, entry.key())? {
                            entry.insert(row);
                        }
                    }
                }
            }
        }
        // Seeds are the retrieval result proper; anchor them even if the lanes above capped them out.
        for seed in seeds {
            if let Some(row) = index::query_one(&self.conn, &seed.frontmatter.id)? {
                selected.entry(seed.frontmatter.id.clone()).or_insert(row);
            }
        }
        neigh.stats.base_candidates = selected.len();

        // Expand the edge-neighbourhood outward. `MAX_HOPS` rings is exactly how far activation can
        // travel, so this holds every node the spread can reach — no more.
        let mut frontier: Vec<String> = selected.keys().cloned().collect();
        for _ in 0..MAX_HOPS {
            let mut next: Vec<String> = Vec::new();
            for id in &frontier {
                neigh.stats.neighbor_expansions += 1;
                neigh.expand(id, &mut selected, &mut next)?;
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        neigh.stats.corpus_nodes = selected.len();
        let vectors: HashMap<String, Vec<f32>> = neigh
            .vectors
            .iter()
            .filter(|(id, _)| selected.contains_key(id))
            .cloned()
            .collect();
        let rows: Vec<IndexRow> = selected.into_values().collect();
        Ok(BoundedCorpus {
            rows,
            vectors,
            hub_tags: neigh.hub_tags,
            stats: neigh.stats,
        })
    }

    /// Semantic neighbours: for each active memory, up to `top_k` others whose embedding cosine is
    /// at least `min_sim`. Each pair appears once. Empty when the store has no real (non-hash)
    /// embeddings — meaning-based links need a semantic backend.
    pub fn related(&self, top_k: usize, min_sim: f32) -> Result<Vec<(String, String, f32)>, Error> {
        let vecs = index::vectors_for(
            &self.conn,
            &self
                .list()?
                .into_iter()
                .filter(|r| r.status == "active")
                .map(|r| r.id)
                .collect::<Vec<_>>(),
        )?;
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        // Each node's nearest neighbours come from the vector top-k primitive — a pgvector ANN query
        // on a hosted backend — rather than a hand-rolled cosine against every other vector. `+1`
        // absorbs the node's own vector (cosine 1.0); the rest are its true neighbours.
        for (id, vector) in &vecs {
            for (target, sim) in self.vector_candidates(vector, top_k + 1, &Query::default())? {
                if &target == id || sim < min_sim {
                    continue;
                }
                let (lo, hi) = if id < &target {
                    (id.clone(), target)
                } else {
                    (target, id.clone())
                };
                if seen.insert((lo.clone(), hi.clone())) {
                    edges.push((lo, hi, sim));
                }
            }
        }
        Ok(edges)
    }

    /// Configured response-rendering mode (full bodies vs terse budgeted lines).
    pub fn retrieval_mode(&self) -> crate::config::ResponseMode {
        self.config.retrieval.response_mode
    }

    /// Hybrid search: keyword (FTS5/BM25) fused with semantic (vector cosine) via weighted
    /// RRF. `hybrid_weight` of 0 (or no embedder) is exactly keyword search.
    ///
    /// `topic` and `tag` are HINTS here, never filters, for the same reason `area` is only a
    /// boost: a caller searching by text is guessing, and a wrong guess must cost it ranking,
    /// never visibility. Answering "I know nothing" while holding the answer is the one failure
    /// a memory store cannot afford — the caller acts on the silence. Scope stays hard:
    /// `project_id`, `kind`, `status` and confidence still exclude, because those are contracts
    /// about what may be seen rather than opinions about what is relevant. Callers who want
    /// exact matching have [`Store::query`], where filtering is the whole point.
    pub fn search(&self, text: &str, q: &Query) -> Result<Vec<Memory>, Error> {
        let now = util::now_rfc3339();
        // Fold the hints into the query text so they widen retrieval and feed the topic-tier
        // reranker, then drop them from the structured filter so they cannot exclude anything.
        let hints: Vec<&str> = [q.topic.as_deref(), q.tag.as_deref()]
            .into_iter()
            .flatten()
            .filter(|h| !h.is_empty())
            .collect();
        let seek = if hints.is_empty() {
            text.to_string()
        } else {
            format!("{text} {}", hints.join(" "))
        };
        let scoped = Query {
            topic: None,
            tag: None,
            ..q.clone()
        };

        // Keyword lane: bm25-ordered candidates, then topic-tier reranked so a strong topic
        // match outranks a merely body-dense one.
        let candidates = index::search(&self.conn, &seek, &scoped, &now)?;
        let keyword: Vec<String> = crate::rank::rerank(candidates, &seek)
            .into_iter()
            .map(|r| r.id)
            .collect();

        let w = q
            .hybrid_weight
            .unwrap_or(self.config.embedding.default_weight);
        let semantic = match (&self.embedder, w > 0.0) {
            (Some(embedder), true) => {
                self.semantic_ranking(&seek, &scoped, embedder.as_ref(), &now)?
            }
            _ => Vec::new(),
        };

        let fused = rrf(&keyword, &semantic, w);
        let rows: Vec<IndexRow> = fused
            .into_iter()
            .filter_map(|id| index::query_one(&self.conn, &id).ok().flatten())
            .collect();
        self.load_budgeted(rows, q.max_tokens)
    }

    /// The query's semantic lane: the `SEMANTIC_TOP_K` memories nearest the query embedding, honouring
    /// the structured filter. It consumes a bounded, FILTER-AWARE vector top-k from
    /// [`Store::vector_candidates`] — the pgvector-ANN seam — so the filter is applied BEFORE the
    /// top-k, exactly as the old filter-then-cosine path did. (Post-filtering a global top-k would
    /// drop matching-but-lower-cosine rows.)
    fn semantic_ranking(
        &self,
        text: &str,
        q: &Query,
        embedder: &dyn Embedder,
        _now: &str,
    ) -> Result<Vec<String>, Error> {
        let qvec = embedder
            .embed_one(text)
            .map_err(|e| Error::Db(e.to_string()))?;
        let ranked = self
            .vector_candidates(&qvec, SEMANTIC_TOP_K, q)?
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .map(|(id, _)| id)
            .collect();
        Ok(ranked)
    }

    fn load_budgeted(
        &self,
        rows: Vec<IndexRow>,
        max_tokens: Option<usize>,
    ) -> Result<Vec<Memory>, Error> {
        let mut out = Vec::new();
        let mut used = 0usize;
        for row in rows {
            let memory = self.hydrate(&row)?;
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
        let active = index::query(
            &self.conn,
            &Query {
                status: Some(marrow_memdocs::Status::Active),
                ..Query::default()
            },
            &util::now_rfc3339(),
        )?;
        for row in active {
            hits.extend(check_memory(repo_root, &self.hydrate(&row)?));
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
            doc: to_markdown(memory),
        }
    }
}

/// Write `text` to `path` atomically (temp file in the same dir, then rename).
/// Upper bound on semantic candidates fed into fusion, so a `w=1` search never returns the
/// whole store.
const SEMANTIC_TOP_K: usize = 64;

/// Default size of the bounded recall candidate set (keyword lane + vector lane, split evenly).
/// Comfortably above `k * fan-out` for realistic `k`, so the neighbourhood the spread walks is fully
/// captured, yet a fixed constant — the work never grows with the store.
pub(crate) const RECALL_CANDIDATES: usize = 128;

/// Cap on how many same-topic rows one node contributes to the bounded corpus. Every realistic
/// topic cluster fits well under this, so edges are captured exactly; it exists only so a single
/// pathological super-topic cannot, by itself, pull an unbounded crowd into the corpus.
const NEIGHBOR_TOPIC_CAP: usize = 256;

/// Instrumentation for the bounded recall corpus build: proof, in a test, that the work is
/// `O(candidates + neighbourhood)` and independent of store size — never a full scan. Read only by
/// the bounds test; production recall ignores it.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub(crate) struct BoundedStats {
    /// Rows in the base candidate set before edge expansion.
    pub base_candidates: usize,
    /// Total rows in the assembled corpus (base + neighbourhood).
    pub corpus_nodes: usize,
    /// Nodes expanded (one `expand` call each), bounded by the spread's fan-out.
    pub neighbor_expansions: usize,
    /// Distinct topic stars queried (memoised — one query per topic, not per node).
    pub topic_queries: usize,
    /// Distinct tag stars queried (memoised — one query per tag, not per node).
    pub tag_queries: usize,
    /// Whole active-vector-set loads. Must stay `<= 1` per recall — the local ANN seam is read once.
    pub full_vector_loads: usize,
    /// `keyword_candidates` (FTS) invocations issued while assembling the corpus.
    pub keyword_candidate_calls: usize,
}

/// The bounded recall corpus: the candidate rows, their embeddings, the global hub tags whose edges
/// must be suppressed, and how it was built.
pub(crate) struct BoundedCorpus {
    pub rows: Vec<IndexRow>,
    pub vectors: HashMap<String, Vec<f32>>,
    /// Tags that are global hubs (more than the recall fan-out of members store-wide). Passed to
    /// [`EdgeCorpus`] so a hub tag whose members only partially enter the bounded corpus does not
    /// present as a small, non-hub tag and emit edges the full scan suppresses.
    pub hub_tags: std::collections::HashSet<String>,
    /// Build instrumentation, read only by the bounds test.
    #[allow(dead_code)]
    pub stats: BoundedStats,
}

/// Per-recall neighbourhood loader. It amortises the bounded corpus build across the many nodes the
/// spread touches: the active vector set is loaded once, every topic/tag star is memoised the first
/// time it is asked for, and global hub tags are recorded as they are discovered. A dense store
/// therefore costs one vector load plus one index query per DISTINCT topic/tag — not one per node —
/// while every lookup stays a bounded index read (the FTS/pgvector seam), never a whole-store scan.
struct Neighborhood<'a> {
    store: &'a Store,
    now: String,
    /// Every active memory's embedding, loaded once (empty when the store has no vectors).
    vectors: Vec<(String, Vec<f32>)>,
    topic_star: HashMap<String, Vec<IndexRow>>,
    /// `None` marks a global hub tag: its star is never pulled and its edges are suppressed.
    tag_star: HashMap<String, Option<Vec<IndexRow>>>,
    hub_tags: std::collections::HashSet<String>,
    stats: BoundedStats,
}

impl<'a> Neighborhood<'a> {
    fn new(store: &'a Store) -> Result<Self, Error> {
        let vectors = store.vectors()?; // the single whole-active-vector load per recall
        Ok(Self {
            store,
            now: util::now_rfc3339(),
            vectors,
            topic_star: HashMap::new(),
            tag_star: HashMap::new(),
            hub_tags: std::collections::HashSet::new(),
            stats: BoundedStats {
                full_vector_loads: 1,
                ..BoundedStats::default()
            },
        })
    }

    /// Top-k active ids nearest `qvec` over the once-loaded vector set — the base vector lane.
    fn query_vector_candidates(&self, qvec: &[f32], top_k: usize) -> Vec<String> {
        let mut scored: Vec<(&str, f32)> = self
            .vectors
            .iter()
            .map(|(id, v)| (id.as_str(), cosine(qvec, v)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored
            .into_iter()
            .take(top_k)
            .map(|(id, _)| id.to_string())
            .collect()
    }

    /// Add `id`'s edge-neighbourhood — full topic star, non-hub tag stars, ref targets, top-k semantic
    /// neighbours — into `selected`, pushing any newly-seen ids onto `next`. These are exactly the
    /// rows [`EdgeCorpus::edges_from`] draws `id`'s edges from, so the assembled corpus reproduces the
    /// full-scan edges for `id`.
    fn expand(
        &mut self,
        id: &str,
        selected: &mut std::collections::BTreeMap<String, IndexRow>,
        next: &mut Vec<String>,
    ) -> Result<(), Error> {
        let Some(node) = index::query_one(&self.store.conn, id)? else {
            return Ok(());
        };
        if node.status != "active" {
            return Ok(());
        }

        if !node.topic.is_empty() {
            self.ensure_topic(&node.topic)?;
            if let Some(members) = self.topic_star.get(&node.topic) {
                add_rows(members, selected, next);
            }
        }

        for tag in crate::edge::tags_of(&node.tags) {
            self.ensure_tag(tag)?;
            if let Some(Some(members)) = self.tag_star.get(tag) {
                add_rows(members, selected, next);
            }
        }

        for value in crate::edge::wiki_refs(&node.body)
            .into_iter()
            .chain(crate::edge::wiki_refs(&node.topic))
        {
            if let Some(row) = self.resolve_ref(&value)? {
                add_rows(std::slice::from_ref(&row), selected, next);
            }
        }

        for target in self.semantic_neighbors(id) {
            if let std::collections::btree_map::Entry::Vacant(entry) = selected.entry(target) {
                if let Some(row) = index::query_one(&self.store.conn, entry.key())? {
                    next.push(entry.key().clone());
                    entry.insert(row);
                }
            }
        }
        Ok(())
    }

    /// Memoise `topic`'s full active-member star (capped so one super-topic can't pull an unbounded
    /// crowd into the corpus).
    fn ensure_topic(&mut self, topic: &str) -> Result<(), Error> {
        if self.topic_star.contains_key(topic) {
            return Ok(());
        }
        self.stats.topic_queries += 1;
        let q = Query {
            topic: Some(topic.to_string()),
            status: Some(Status::Active),
            limit: Some(NEIGHBOR_TOPIC_CAP),
            ..Query::default()
        };
        let rows = index::query(&self.store.conn, &q, &self.now)?;
        self.topic_star.insert(topic.to_string(), rows);
        Ok(())
    }

    /// Memoise `tag`'s member star, or record it as a global hub (`None`) when it exceeds the recall
    /// fan-out — the same gate `edges_from` applies, but on GLOBAL membership.
    fn ensure_tag(&mut self, tag: &str) -> Result<(), Error> {
        if self.tag_star.contains_key(tag) {
            return Ok(());
        }
        self.stats.tag_queries += 1;
        let q = Query {
            tag: Some(tag.to_string()),
            status: Some(Status::Active),
            // One past the fan-out is enough to tell a normal tag from a hub tag.
            limit: Some(crate::edge::RECALL_TAG_FANOUT + 1),
            ..Query::default()
        };
        let members = index::query(&self.store.conn, &q, &self.now)?;
        let entry = if members.len() > crate::edge::RECALL_TAG_FANOUT {
            self.hub_tags.insert(tag.to_string());
            None
        } else {
            Some(members)
        };
        self.tag_star.insert(tag.to_string(), entry);
        Ok(())
    }

    /// Resolve a `[[…]]` reference the way [`EdgeCorpus::resolve_ref`] does: an exact id first, then
    /// the first active memory carrying that topic.
    fn resolve_ref(&self, value: &str) -> Result<Option<IndexRow>, Error> {
        if let Some(row) = index::query_one(&self.store.conn, value)? {
            return Ok(Some(row));
        }
        let q = Query {
            topic: Some(value.to_string()),
            status: Some(Status::Active),
            limit: Some(1),
            ..Query::default()
        };
        Ok(index::query(&self.store.conn, &q, &self.now)?
            .into_iter()
            .next())
    }

    /// `id`'s top-k semantic neighbour ids over the once-loaded vector set, selected byte-for-byte the
    /// way [`EdgeCorpus`] does (cosine `>= SEMANTIC_MIN_SIM`, id tiebreak, `RECALL_SEMANTIC_TOP_K`), so
    /// pulling them guarantees the corpus reconstructs `id`'s true semantic edges.
    fn semantic_neighbors(&self, id: &str) -> Vec<String> {
        let Some(source) = self
            .vectors
            .iter()
            .find(|(vid, _)| vid == id)
            .map(|(_, v)| v)
        else {
            return Vec::new();
        };
        let mut scored: Vec<(&str, f32)> = self
            .vectors
            .iter()
            .filter(|(vid, v)| vid != id && v.len() == source.len())
            .map(|(vid, v)| (vid.as_str(), cosine(source, v)))
            .filter(|(_, sim)| *sim >= crate::edge::SEMANTIC_MIN_SIM)
            .collect();
        scored.sort_by(|(a_id, a), (b_id, b)| b.total_cmp(a).then_with(|| a_id.cmp(b_id)));
        scored
            .into_iter()
            .take(crate::edge::RECALL_SEMANTIC_TOP_K)
            .map(|(id, _)| id.to_string())
            .collect()
    }
}

/// Insert any not-yet-seen `rows` into `selected`, recording their ids on `next` for the next hop.
fn add_rows(
    rows: &[IndexRow],
    selected: &mut std::collections::BTreeMap<String, IndexRow>,
    next: &mut Vec<String>,
) {
    for row in rows {
        if let std::collections::btree_map::Entry::Vacant(entry) = selected.entry(row.id.clone()) {
            next.push(entry.key().clone());
            entry.insert(row.clone());
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use marrow_memdocs::{Decay, Frontmatter, MemoryKind, Provenance, Scope, Status};

    /// A memory exercising every frontmatter field that must round-trip through the index row.
    fn rich_memory() -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind: MemoryKind::Decision,
                status: Status::Active,
                topic: Some("auth strategy".into()),
                area: Some("auth".into()),
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![Ref {
                    kind: RefKind::Url,
                    value: "https://example.com/rfc".into(),
                }],
                code_anchors: vec![CodeAnchor {
                    file_path: "src/auth.rs".into(),
                    symbol: "login".into(),
                    snippet: "fn login() {}".into(),
                    fingerprint: "abc123".into(),
                    norm: "fn login".into(),
                }],
                confidence: 0.75,
                decay: Some(Decay {
                    expires_at: Some("2027-01-01T00:00:00Z".into()),
                }),
                provenance: Provenance {
                    written_by: "claude-code".into(),
                    model: Some("claude-opus-4-8".into()),
                    session_id: Some("sess-42".into()),
                    sources: vec!["chat".into(), "rfc".into()],
                },
                supersedes: vec!["01OLD".into()],
                tags: vec!["security".into(), "jwt".into()],
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: "We use JWT for sessions.\n\nRotated every 24h.\n".into(),
        }
    }

    #[test]
    fn read_serves_from_row_not_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let mut m = rich_memory();
        let id = store.write(&mut m).unwrap();

        // Delete every exported markdown file, proving the read cannot touch disk.
        fs::remove_dir_all(store.memory_dir()).unwrap();
        assert!(markdown_files(&store.memory_dir()).is_empty());

        let got = store.read(&id).unwrap().expect("row hydration");
        assert_eq!(got, m);
    }

    #[test]
    fn write_exports_markdown_matching_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let mut m = rich_memory();
        let id = store.write(&mut m).unwrap();

        // The write-through export exists on disk...
        let files = markdown_files(&store.memory_dir());
        assert_eq!(files.len(), 1, "exactly one exported markdown file");

        // ...and re-parsing it yields exactly the row-hydrated memory.
        let from_disk = marrow_memdocs::parse(&fs::read_to_string(&files[0]).unwrap()).unwrap();
        let from_row = store.read(&id).unwrap().expect("row hydration");
        assert_eq!(from_disk, from_row);
        assert_eq!(from_disk, m);
    }

    #[test]
    fn signed_memory_still_verifies_after_row_hydration() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::init(dir.path()).unwrap();
        store.config.sign = true;
        store.set_key(b"secret-key".to_vec());

        let mut m = rich_memory();
        let id = store.write(&mut m).unwrap();

        // Reads served from the row must reconstruct a Memory whose HMAC still verifies — the whole
        // point of preserving exact fidelity so downstream signing stays valid.
        let got = store.read(&id).unwrap().expect("row hydration");
        assert_eq!(got, m);
        assert_eq!(store.verify(&got), Some(true));
    }
}
