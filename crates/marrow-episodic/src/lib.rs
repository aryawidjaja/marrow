//! An append-only, hash-chained event ledger.
//!
//! Every event records the hash of the previous one, forming a tamper-evident chain: edit
//! or remove any past entry and [`EpisodicLog::verify`] reports the break. This is both the
//! episodic memory (a raw, timestamped record that is never rewritten) and the audit trail.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// A new event to append. `seq`, timestamp, and hashes are filled in by the log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEvent {
    /// What happened: "write", "supersede", "observe", "correct", ...
    pub kind: String,
    /// Who caused it.
    pub actor: String,
    /// The memory this concerns, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    /// A short human-readable description.
    pub summary: String,
    /// Event-specific structured payload.
    #[serde(default)]
    pub data: serde_json::Value,
}

impl NewEvent {
    pub fn new(kind: &str, actor: &str, summary: &str) -> Self {
        NewEvent {
            kind: kind.to_string(),
            actor: actor.to_string(),
            memory_id: None,
            summary: summary.to_string(),
            data: serde_json::Value::Null,
        }
    }

    pub fn memory(mut self, id: &str) -> Self {
        self.memory_id = Some(id.to_string());
        self
    }
}

/// A recorded event, with its position, timestamp, and chain hashes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub ts: String,
    pub kind: String,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub data: serde_json::Value,
    pub content_hash: String,
    pub prev_hash: String,
}

/// Errors from the ledger.
#[derive(Debug)]
pub enum Error {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(m) => write!(f, "episodic io error: {m}"),
            Error::Parse(m) => write!(f, "episodic parse error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// An append-only ledger backed by a single JSONL file.
pub struct EpisodicLog {
    path: PathBuf,
    last_seq: u64,
    last_hash: String,
}

impl EpisodicLog {
    /// Open (creating if needed) the ledger at `<root>/.marrow/episodic/log.jsonl`, recovering
    /// the chain tip so appends continue the chain.
    pub fn open(root: impl AsRef<Path>) -> Result<EpisodicLog, Error> {
        let dir = root.as_ref().join(".marrow/episodic");
        fs::create_dir_all(&dir)?;
        let path = dir.join("log.jsonl");
        let (last_seq, last_hash) = match read_events(&path)?.last() {
            Some(e) => (e.seq, e.content_hash.clone()),
            None => (0, String::new()),
        };
        Ok(EpisodicLog {
            path,
            last_seq,
            last_hash,
        })
    }

    /// Append an event, returning the stored record.
    pub fn append(&mut self, ev: NewEvent) -> Result<Event, Error> {
        let seq = self.last_seq + 1;
        let ts = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());
        let content_hash = content_hash(seq, &ts, &ev);
        let event = Event {
            seq,
            ts,
            kind: ev.kind,
            actor: ev.actor,
            memory_id: ev.memory_id,
            summary: ev.summary,
            data: ev.data,
            content_hash: content_hash.clone(),
            prev_hash: self.last_hash.clone(),
        };
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        let line = serde_json::to_string(&event).map_err(|e| Error::Parse(e.to_string()))?;
        writeln!(file, "{line}")?;
        self.last_seq = seq;
        self.last_hash = content_hash;
        Ok(event)
    }

    /// All events in order.
    pub fn read_all(&self) -> Result<Vec<Event>, Error> {
        read_events(&self.path)
    }

    /// Events with `seq` greater than `after`.
    pub fn since(&self, after: u64) -> Result<Vec<Event>, Error> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|e| e.seq > after)
            .collect())
    }

    /// Verify the chain. `Ok(())` if intact, else the `seq` of the first broken entry.
    pub fn verify(&self) -> Result<(), u64> {
        let events = self.read_all().map_err(|_| 0u64)?;
        let mut prev = String::new();
        for (i, e) in events.iter().enumerate() {
            let expected_seq = i as u64 + 1;
            let recomputed = content_hash(
                e.seq,
                &e.ts,
                &NewEvent {
                    kind: e.kind.clone(),
                    actor: e.actor.clone(),
                    memory_id: e.memory_id.clone(),
                    summary: e.summary.clone(),
                    data: e.data.clone(),
                },
            );
            if e.seq != expected_seq || e.prev_hash != prev || e.content_hash != recomputed {
                return Err(e.seq);
            }
            prev = e.content_hash.clone();
        }
        Ok(())
    }
}

/// SHA-256 over the canonical content of an event (everything but the chain hashes).
fn content_hash(seq: u64, ts: &str, ev: &NewEvent) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        seq: u64,
        ts: &'a str,
        kind: &'a str,
        actor: &'a str,
        memory_id: &'a Option<String>,
        summary: &'a str,
        data: &'a serde_json::Value,
    }
    let canonical = Canonical {
        seq,
        ts,
        kind: &ev.kind,
        actor: &ev.actor,
        memory_id: &ev.memory_id,
        summary: &ev.summary,
        data: &ev.data,
    };
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hex::encode(hasher.finalize())
}

fn read_events(path: &Path) -> Result<Vec<Event>, Error> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Event = serde_json::from_str(&line).map_err(|e| Error::Parse(e.to_string()))?;
        out.push(event);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_chains_and_increments_seq() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        let a = log.append(NewEvent::new("write", "agent", "stored fact")).unwrap();
        let b = log.append(NewEvent::new("write", "agent", "stored decision")).unwrap();
        assert_eq!(a.seq, 1);
        assert_eq!(b.seq, 2);
        assert_eq!(a.prev_hash, "");
        assert_eq!(b.prev_hash, a.content_hash);
    }

    #[test]
    fn verify_passes_for_intact_chain() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        for i in 0..5 {
            log.append(NewEvent::new("observe", "agent", &format!("event {i}"))).unwrap();
        }
        assert_eq!(log.verify(), Ok(()));
    }

    #[test]
    fn verify_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        log.append(NewEvent::new("write", "agent", "original")).unwrap();
        log.append(NewEvent::new("write", "agent", "second")).unwrap();

        // Tamper with the first line's summary directly on disk.
        let path = dir.path().join(".marrow/episodic/log.jsonl");
        let content = fs::read_to_string(&path).unwrap();
        let tampered = content.replacen("original", "forged", 1);
        fs::write(&path, tampered).unwrap();

        assert_eq!(log.verify(), Err(1));
    }

    #[test]
    fn persists_and_continues_chain_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut log = EpisodicLog::open(dir.path()).unwrap();
            log.append(NewEvent::new("write", "agent", "first")).unwrap();
        }
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        let next = log.append(NewEvent::new("write", "agent", "second")).unwrap();
        assert_eq!(next.seq, 2);
        assert_eq!(log.read_all().unwrap().len(), 2);
        assert_eq!(log.verify(), Ok(()));
    }

    #[test]
    fn since_returns_only_newer_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        log.append(NewEvent::new("a", "x", "one")).unwrap();
        log.append(NewEvent::new("b", "x", "two")).unwrap();
        log.append(NewEvent::new("c", "x", "three")).unwrap();
        let recent = log.since(1).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].seq, 2);
    }

    #[test]
    fn carries_memory_id_and_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        let mut ev = NewEvent::new("write", "agent", "stored").memory("01ABC");
        ev.data = serde_json::json!({"topic": "auth"});
        let stored = log.append(ev).unwrap();
        assert_eq!(stored.memory_id.as_deref(), Some("01ABC"));
        assert_eq!(stored.data["topic"], "auth");
    }
}
