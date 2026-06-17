//! The memory store: markdown files as the source of truth, SQLite as a derived index.

use std::fs;
use std::path::{Path, PathBuf};

use marrow_memdocs::{to_markdown, validate, Memory, Violation};
use rusqlite::Connection;
use ulid::Ulid;

use crate::config::Config;
use crate::convert::{kind_str, status_str};
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
    config: Config,
    key: Option<Vec<u8>>,
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
        let root = root.as_ref().to_path_buf();
        let marrow = root.join(".marrow");
        let config = match fs::read_to_string(marrow.join(".marrow.toml")) {
            Ok(t) => Config::from_toml(&t).map_err(|e| Error::Parse(e.to_string()))?,
            Err(_) => Config::default(),
        };
        fs::create_dir_all(marrow.join(".index"))?;
        let conn = Connection::open(marrow.join(".index/sqlite.db"))?;
        index::init_schema(&conn)?;
        let key = if config.sign {
            std::env::var("MARROW_HMAC_KEY")
                .ok()
                .map(|s| s.into_bytes())
        } else {
            None
        };
        Ok(Store {
            root,
            conn,
            config,
            key,
        })
    }

    /// Override the signing key (mainly for tests and embedding).
    pub fn set_key(&mut self, key: impl Into<Vec<u8>>) {
        self.key = Some(key.into());
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

        index::upsert(&self.conn, &self.row_of(memory, &rel))?;
        Ok(memory.frontmatter.id.clone())
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

    /// Structured query, loading full memories under an optional token budget.
    pub fn query(&self, q: &Query) -> Result<Vec<Memory>, Error> {
        let rows = index::query(&self.conn, q, &util::now_rfc3339())?;
        self.load_budgeted(rows, q.max_tokens)
    }

    /// Full-text search with the same structured filters and token budget.
    pub fn search(&self, text: &str, q: &Query) -> Result<Vec<Memory>, Error> {
        let rows = index::search(&self.conn, text, q, &util::now_rfc3339())?;
        self.load_budgeted(rows, q.max_tokens)
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
        self.write(&mut old)?;
        if !new.frontmatter.supersedes.iter().any(|s| s == old_id) {
            new.frontmatter.supersedes.push(old_id.to_string());
        }
        self.write(new)
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
            count += 1;
        }
        Ok(count)
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
            project_id: fm.scope.project_id.clone(),
            user_id: fm.scope.user_id.clone().unwrap_or_default(),
            agent_id: fm.scope.agent_id.clone().unwrap_or_default(),
            org_id: fm.scope.org_id.clone().unwrap_or_default(),
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
