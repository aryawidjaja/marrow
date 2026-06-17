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
    pub project_id: String,
    pub user_id: String,
    pub agent_id: String,
    pub org_id: String,
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
            project_id TEXT NOT NULL DEFAULT '',
            user_id TEXT NOT NULL DEFAULT '',
            agent_id TEXT NOT NULL DEFAULT '',
            org_id TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL DEFAULT 1.0,
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            expires_at TEXT NOT NULL DEFAULT '',
            tags TEXT NOT NULL DEFAULT '',
            path TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT ''
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            id UNINDEXED, topic, body, tags
        );
        CREATE TABLE IF NOT EXISTS embeddings (
            id TEXT PRIMARY KEY,
            dim INTEGER NOT NULL,
            vec BLOB NOT NULL
        );",
    )
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
         (id, kind, status, topic, project_id, user_id, agent_id, org_id,
          confidence, created_at, updated_at, expires_at, tags, path, body)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            row.id,
            row.kind,
            row.status,
            row.topic,
            row.project_id,
            row.user_id,
            row.agent_id,
            row.org_id,
            row.confidence,
            row.created_at,
            row.updated_at,
            row.expires_at,
            row.tags,
            row.path,
            row.body
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
    conn.execute_batch("DELETE FROM memories; DELETE FROM memories_fts; DELETE FROM embeddings;")
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
    eq("user_id", q.user_id.as_ref(), &mut binds, &mut clauses);
    eq("agent_id", q.agent_id.as_ref(), &mut binds, &mut clauses);
    eq("org_id", q.org_id.as_ref(), &mut binds, &mut clauses);
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

const COLS: &str = "id,kind,status,topic,project_id,user_id,agent_id,org_id,confidence,created_at,updated_at,expires_at,tags,path,body";

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<IndexRow> {
    Ok(IndexRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        status: r.get(2)?,
        topic: r.get(3)?,
        project_id: r.get(4)?,
        user_id: r.get(5)?,
        agent_id: r.get(6)?,
        org_id: r.get(7)?,
        confidence: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
        expires_at: r.get(11)?,
        tags: r.get(12)?,
        path: r.get(13)?,
        body: r.get(14)?,
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

/// Full-text search via FTS5, with the same structured filters applied.
pub fn search(
    conn: &Connection,
    text: &str,
    q: &Query,
    now: &str,
) -> rusqlite::Result<Vec<IndexRow>> {
    let (mut where_sql, mut binds) = filters(q, now);
    // Prepend the FTS MATCH constraint.
    if where_sql.is_empty() {
        where_sql = "WHERE m.id IN (SELECT id FROM memories_fts WHERE memories_fts MATCH ?)".into();
        binds.insert(0, text.to_string());
    } else {
        where_sql = where_sql.replacen(
            "WHERE ",
            "WHERE m.id IN (SELECT id FROM memories_fts WHERE memories_fts MATCH ?) AND ",
            1,
        );
        binds.insert(0, text.to_string());
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
