//! An append-only, hash-chained event ledger.
//!
//! Every event records the hash of the previous one, forming a tamper-evident chain: edit
//! or remove any past entry and [`EpisodicLog::verify`] reports the break. This is both the
//! episodic memory (a raw, timestamped record that is never rewritten) and the audit trail.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
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
///
/// Appends are serialized across threads and processes with an advisory file lock, and the chain
/// tip (`seq` + `prev_hash`) is re-read from disk under that lock on every append — so two sessions
/// writing at once can't interleave lines or collide on a sequence number.
pub struct EpisodicLog {
    path: PathBuf,
}

impl EpisodicLog {
    /// Open (creating the directory if needed) the ledger at `<root>/.marrow/episodic/log.jsonl`.
    pub fn open(root: impl AsRef<Path>) -> Result<EpisodicLog, Error> {
        let dir = root.as_ref().join(".marrow/episodic");
        fs::create_dir_all(&dir)?;
        Ok(EpisodicLog {
            path: dir.join("log.jsonl"),
        })
    }

    /// Append an event, returning the stored record. Holds an exclusive lock for the whole
    /// read-tip-then-write so concurrent writers serialize instead of corrupting the log.
    pub fn append(&mut self, ev: NewEvent) -> Result<Event, Error> {
        let lock = self.lock(true)?;

        let (last_seq, last_hash) = match read_tip(&self.path)? {
            Some(e) => (e.seq, e.content_hash),
            None => (0, String::new()),
        };
        let seq = last_seq + 1;
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
            content_hash,
            prev_hash: last_hash,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(&event).map_err(|e| Error::Parse(e.to_string()))?;
        // One write of the whole framed line, under the lock — never a torn or interleaved line.
        file.write_all(format!("{line}\n").as_bytes())?;
        file.flush()?;
        FileExt::unlock(&lock).ok();
        Ok(event)
    }

    /// Open and lock the sidecar lock file (`log.lock`). Exclusive for writers, shared for readers,
    /// so a read never observes a half-written line.
    fn lock(&self, exclusive: bool) -> Result<File, Error> {
        let lock_path = self.path.with_extension("lock");
        let f = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;
        if exclusive {
            f.lock_exclusive()?;
        } else {
            f.lock_shared()?;
        }
        Ok(f)
    }

    /// All events in order.
    pub fn read_all(&self) -> Result<Vec<Event>, Error> {
        let lock = self.lock(false)?;
        let out = read_events_raw(&self.path);
        FileExt::unlock(&lock).ok();
        out
    }

    /// Events with `seq` greater than `after`.
    pub fn since(&self, after: u64) -> Result<Vec<Event>, Error> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|e| e.seq > after)
            .collect())
    }

    /// The newest `limit` events, oldest first. Reads backward from EOF instead of parsing the
    /// whole ledger.
    pub fn tail(&self, limit: usize) -> Result<Vec<Event>, Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let lock = self.lock(false)?;
        let out = read_tail(&self.path, limit);
        FileExt::unlock(&lock).ok();
        out
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

/// Size of each backward read when scanning from EOF.
const TAIL_CHUNK: usize = 64 * 1024;

/// Collect complete lines from the end of `path`, newest first, until `want` are found or the file
/// is exhausted. Returns them in file order.
fn read_lines_from_end(path: &Path, want: usize) -> Result<Vec<String>, Error> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut pos = file.seek(SeekFrom::End(0))?;
    let mut pending: Vec<u8> = Vec::new();
    let mut lines: Vec<String> = Vec::new();

    while pos > 0 && lines.len() < want {
        let step = TAIL_CHUNK.min(pos as usize);
        pos -= step as u64;
        file.seek(SeekFrom::Start(pos))?;
        let mut buf = vec![0u8; step];
        file.read_exact(&mut buf)?;
        buf.extend_from_slice(&pending);

        let mut cut = buf.len();
        while lines.len() < want {
            match buf[..cut].iter().rposition(|&b| b == b'\n') {
                Some(nl) => {
                    let line = String::from_utf8_lossy(&buf[nl + 1..cut]).into_owned();
                    if !line.trim().is_empty() {
                        lines.push(line);
                    }
                    cut = nl;
                }
                None => break,
            }
        }
        pending = buf[..cut].to_vec();
    }
    if pos == 0 && lines.len() < want {
        let line = String::from_utf8_lossy(&pending).into_owned();
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }
    lines.reverse();
    Ok(lines)
}

/// The last recorded event, or `None` for an empty ledger.
fn read_tip(path: &Path) -> Result<Option<Event>, Error> {
    let Some(line) = read_lines_from_end(path, 1)?.pop() else {
        return Ok(None);
    };
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| Error::Parse(e.to_string()))
}

fn read_tail(path: &Path, limit: usize) -> Result<Vec<Event>, Error> {
    read_lines_from_end(path, limit)?
        .iter()
        .map(|l| serde_json::from_str(l).map_err(|e| Error::Parse(e.to_string())))
        .collect()
}

fn read_events_raw(path: &Path) -> Result<Vec<Event>, Error> {
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
        let a = log
            .append(NewEvent::new("write", "agent", "stored fact"))
            .unwrap();
        let b = log
            .append(NewEvent::new("write", "agent", "stored decision"))
            .unwrap();
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
            log.append(NewEvent::new("observe", "agent", &format!("event {i}")))
                .unwrap();
        }
        assert_eq!(log.verify(), Ok(()));
    }

    #[test]
    fn verify_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        log.append(NewEvent::new("write", "agent", "original"))
            .unwrap();
        log.append(NewEvent::new("write", "agent", "second"))
            .unwrap();

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
            log.append(NewEvent::new("write", "agent", "first"))
                .unwrap();
        }
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        let next = log
            .append(NewEvent::new("write", "agent", "second"))
            .unwrap();
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
    fn concurrent_appends_never_interleave_or_collide() {
        use std::sync::Arc;
        use std::thread;

        let dir = Arc::new(tempfile::tempdir().unwrap());
        let (writers, per) = (8, 25);

        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let dir = Arc::clone(&dir);
                thread::spawn(move || {
                    // Each thread is its own writer (stands in for a separate process/session).
                    let mut log = EpisodicLog::open(dir.path()).unwrap();
                    for i in 0..per {
                        log.append(NewEvent::new("write", "agent", &format!("w{w} e{i}")))
                            .unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let log = EpisodicLog::open(dir.path()).unwrap();
        let events = log.read_all().unwrap(); // would error on a torn line
        assert_eq!(events.len(), writers * per);

        // Sequence numbers are unique and gap-free (no two writers grabbed the same seq).
        let mut seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        seqs.sort_unstable();
        assert_eq!(seqs, (1..=(writers * per) as u64).collect::<Vec<_>>());

        // And the hash chain is intact.
        assert_eq!(log.verify(), Ok(()));
    }

    #[test]
    fn tail_matches_the_end_of_read_all() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        assert!(log.tail(5).unwrap().is_empty(), "empty ledger");

        for i in 0..250 {
            let mut ev = NewEvent::new("progress", "agent", &format!("event {i}"));
            ev.data = serde_json::json!({ "pad": "x".repeat(600) });
            log.append(ev).unwrap();
        }
        let all = log.read_all().unwrap();

        for n in [1usize, 3, 40, 250, 400] {
            let want: Vec<u64> = all.iter().rev().take(n).rev().map(|e| e.seq).collect();
            let got: Vec<u64> = log.tail(n).unwrap().iter().map(|e| e.seq).collect();
            assert_eq!(got, want, "tail({n}) must match the end of read_all");
        }
        assert!(log.tail(0).unwrap().is_empty());
    }

    #[test]
    fn append_does_not_read_the_whole_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EpisodicLog::open(dir.path()).unwrap();
        let mut ev = NewEvent::new("write", "agent", "seed");
        ev.data = serde_json::json!({ "pad": "y".repeat(2000) });
        for _ in 0..400 {
            log.append(ev.clone()).unwrap();
        }
        // A ledger far larger than one backward chunk still chains correctly from a tip read.
        let path = dir.path().join(".marrow/episodic/log.jsonl");
        assert!(fs::metadata(&path).unwrap().len() > TAIL_CHUNK as u64);
        let next = log
            .append(NewEvent::new("write", "agent", "after"))
            .unwrap();
        assert_eq!(next.seq, 401);
        assert_eq!(log.verify(), Ok(()));
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
