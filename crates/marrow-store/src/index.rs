//! SQLite + FTS5 index over the markdown memory files.
//!
//! The index is a derived, rebuildable cache: delete it and `Store::reindex` rebuilds it
//! from the files on disk. Queries hit the index; the markdown remains the source of truth.

use rusqlite::{params, Connection};

use crate::query::Query;

/// A flattened, indexable view of a memory.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexRow {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub topic: String,
    pub area: String,
    pub project_id: String,
    pub confidence: f64,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub tags: String,
    pub path: String,
    pub body: String,
}

/// Create the schema if absent.
pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            status TEXT NOT NULL,
            topic TEXT NOT NULL DEFAULT '',
            area TEXT NOT NULL DEFAULT '',
            project_id TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 1.0,
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            expires_at TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            path TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT ''
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            id UNINDEXED, topic, body, tags,
            tokenize = 'porter unicode61'
        );
        CREATE TABLE IF NOT EXISTS embeddings (
            id TEXT PRIMARY KEY,
            dim INTEGER NOT NULL,
            vec BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS recalls (
            id TEXT PRIMARY KEY,
            n INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    // Older stores predate `area`; add it in place rather than forcing a full reindex.
    let _ = conn.execute(
        "ALTER TABLE memories ADD COLUMN area TEXT NOT NULL DEFAULT ''",
        [],
    );
    Ok(())
}

/// Count one retrieval against each memory that came back.
pub fn bump_recalls(conn: &Connection, ids: &[String]) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO recalls (id, n) VALUES (?1, 1)
         ON CONFLICT(id) DO UPDATE SET n = n + 1",
    )?;
    for id in ids {
        stmt.execute([id])?;
    }
    Ok(())
}

/// How many times each memory has been recalled. Memories never recalled are absent.
pub fn recall_counts(conn: &Connection) -> rusqlite::Result<Vec<(String, u32)>> {
    let mut stmt = conn.prepare("SELECT id, n FROM recalls WHERE n > 0")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u32)))?;
    rows.collect()
}

/// Replace every recall count, used when rebuilding them from the ledger.
pub fn reset_recalls(conn: &Connection, counts: &[(String, u32)]) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM recalls", [])?;
    let mut stmt = conn.prepare("INSERT OR REPLACE INTO recalls (id, n) VALUES (?1, ?2)")?;
    for (id, n) in counts {
        stmt.execute(params![id, *n as i64])?;
    }
    Ok(())
}

/// Store (or replace) a memory's embedding as a little-endian f32 blob.
pub fn upsert_vector(conn: &Connection, id: &str, vec: &[f32]) -> rusqlite::Result<()> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for x in vec {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (id, dim, vec) VALUES (?1, ?2, ?3)",
        params![id, vec.len() as i64, bytes],
    )?;
    Ok(())
}

/// Load embeddings for the given ids (skipping any with a corrupt/short blob).
pub fn vectors_for(conn: &Connection, ids: &[String]) -> rusqlite::Result<Vec<(String, Vec<f32>)>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare("SELECT dim, vec FROM embeddings WHERE id = ?1")?;
    for id in ids {
        let row = stmt
            .query_row([id], |r| {
                let dim: i64 = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                Ok((dim as usize, blob))
            })
            .ok();
        if let Some((dim, blob)) = row {
            if blob.len() == dim * 4 {
                let v = blob
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                out.push((id.clone(), v));
            }
        }
    }
    Ok(out)
}

/// Insert or replace a row in both the table and the FTS index.
pub fn upsert(conn: &Connection, row: &IndexRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO memories
         (id, kind, status, topic, project_id,
          confidence, created_at, updated_at, expires_at, tags, path, body, area)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            row.id,
            row.kind,
            row.status,
            row.topic,
            row.project_id,
            row.confidence,
            row.created_at,
            row.updated_at,
            row.expires_at,
            row.tags,
            row.path,
            row.body,
            row.area
        ],
    )?;
    conn.execute("DELETE FROM memories_fts WHERE id = ?1", params![row.id])?;
    conn.execute(
        "INSERT INTO memories_fts (id, topic, body, tags) VALUES (?1,?2,?3,?4)",
        params![row.id, row.topic, row.body, row.tags],
    )?;
    Ok(())
}

/// Remove a row by id.
pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
    conn.execute("DELETE FROM memories_fts WHERE id = ?1", params![id])?;
    conn.execute("DELETE FROM embeddings WHERE id = ?1", params![id])?;
    Ok(())
}

/// File path recorded for a memory id.
pub fn path_of(conn: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT path FROM memories WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Remove every row (used before a full reindex).
pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    // Recall counts are derived too — the ledger holds the retrievals they are counted from — so
    // they are wiped with the rest and rebuilt, rather than being allowed to drift forever.
    conn.execute_batch(
        "DELETE FROM memories; DELETE FROM memories_fts; DELETE FROM embeddings; DELETE FROM recalls;",
    )
}

/// Build the shared WHERE clause and bound parameters from a [`Query`].
fn filters(q: &Query, now: &str) -> (String, Vec<String>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    let eq =
        |col: &str, val: Option<&String>, binds: &mut Vec<String>, clauses: &mut Vec<String>| {
            if let Some(v) = val {
                clauses.push(format!("{col} = ?"));
                binds.push(v.clone());
            }
        };
    if let Some(k) = q.kind {
        clauses.push("kind = ?".into());
        binds.push(crate::convert::kind_str(k).into());
    }
    if let Some(s) = q.status {
        clauses.push("status = ?".into());
        binds.push(crate::convert::status_str(s).into());
    }
    eq("topic", q.topic.as_ref(), &mut binds, &mut clauses);
    eq(
        "project_id",
        q.project_id.as_ref(),
        &mut binds,
        &mut clauses,
    );
    if let Some(c) = q.min_confidence {
        clauses.push("confidence >= ?".into());
        binds.push(c.to_string());
    }
    if let Some(t) = &q.tag {
        clauses.push("tags LIKE ?".into());
        binds.push(format!("%,{t},%"));
    }
    if q.exclude_expired {
        clauses.push("(expires_at = '' OR expires_at >= ?)".into());
        binds.push(now.to_string());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (where_sql, binds)
}

const COLS: &str = "id,kind,status,topic,project_id,confidence,created_at,updated_at,expires_at,tags,path,body,area";

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<IndexRow> {
    Ok(IndexRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        status: r.get(2)?,
        topic: r.get(3)?,
        project_id: r.get(4)?,
        confidence: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        expires_at: r.get(8)?,
        tags: r.get(9)?,
        path: r.get(10)?,
        body: r.get(11)?,
        area: r.get(12)?,
    })
}

/// Fetch one indexed row by id (used to load rows in a fused result order).
pub fn query_one(conn: &Connection, id: &str) -> rusqlite::Result<Option<IndexRow>> {
    let sql = format!("SELECT {COLS} FROM memories WHERE id = ?1");
    conn.query_row(&sql, [id], row_from)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
}

/// Structured query.
pub fn query(conn: &Connection, q: &Query, now: &str) -> rusqlite::Result<Vec<IndexRow>> {
    let (where_sql, binds) = filters(q, now);
    let limit = q.limit.map(|n| format!("LIMIT {n}")).unwrap_or_default();
    let sql = format!("SELECT {COLS} FROM memories {where_sql} ORDER BY updated_at DESC {limit}");
    let mut stmt = conn.prepare(&sql)?;
    let params = rusqlite::params_from_iter(binds.iter());
    let rows = stmt.query_map(params, row_from)?;
    rows.collect()
}

/// Turn raw user text into a safe FTS5 MATCH expression: each word becomes a quoted phrase and the
/// phrases are OR-ed. This stops punctuation from being read as FTS5 operators (`.`/`:` → syntax
/// errors) and stops a single missing token from collapsing a multi-word query to zero results
/// (bare FTS5 ANDs terms). Returns an empty string when the query has no searchable words.
fn fts5_query(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Full-text search via FTS5, with the same structured filters applied.
pub fn search(
    conn: &Connection,
    text: &str,
    q: &Query,
    now: &str,
) -> rusqlite::Result<Vec<IndexRow>> {
    // Sanitize the user's text into a safe FTS5 expression. An all-punctuation query has no terms
    // to match, so it returns nothing rather than erroring.
    let match_expr = fts5_query(text);
    if match_expr.is_empty() {
        return Ok(Vec::new());
    }
    let (mut where_sql, mut binds) = filters(q, now);
    // Prepend the FTS MATCH constraint.
    if where_sql.is_empty() {
        where_sql = "WHERE m.id IN (SELECT id FROM memories_fts WHERE memories_fts MATCH ?)".into();
        binds.insert(0, match_expr);
    } else {
        where_sql = where_sql.replacen(
            "WHERE ",
            "WHERE m.id IN (SELECT id FROM memories_fts WHERE memories_fts MATCH ?) AND ",
            1,
        );
        binds.insert(0, match_expr);
    }
    let limit = q.limit.map(|n| format!("LIMIT {n}")).unwrap_or_default();
    let cols: String = COLS
        .split(',')
        .map(|c| format!("m.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql =
        format!("SELECT {cols} FROM memories m {where_sql} ORDER BY m.updated_at DESC {limit}");
    let mut stmt = conn.prepare(&sql)?;
    let params = rusqlite::params_from_iter(binds.iter());
    let rows = stmt.query_map(params, row_from)?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::fts5_query;

    #[test]
    fn fts5_query_quotes_ors_and_drops_punctuation() {
        assert_eq!(
            fts5_query("legend autocount symbol"),
            "\"legend\" OR \"autocount\" OR \"symbol\""
        );
        // Punctuation that FTS5 would treat as operators is dropped, not passed through.
        assert_eq!(fts5_query("E201:"), "\"E201\"");
        assert_eq!(fts5_query("trailing dot."), "\"trailing\" OR \"dot\"");
        assert_eq!(
            fts5_query("first_login_at"),
            "\"first\" OR \"login\" OR \"at\""
        );
        // No searchable words -> empty (caller returns no rows instead of erroring).
        assert_eq!(fts5_query("…"), "");
        assert_eq!(fts5_query("   "), "");
    }
}
