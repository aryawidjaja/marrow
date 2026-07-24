//! Local backend: today's file+SQLite [`Store`] behind the [`MemoryBackend`] port.
//!
//! This is a faithful adapter, not a redesign: every method delegates to the matching [`Store`]
//! call with identical behavior. It ignores `ctx.org`/`ctx.project` (a single local project has no
//! tenant) and sources any required author from `ctx.actor`, defaulting to `"local"`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use marrow_memdocs::Memory;

use crate::backend::{BackendError, Ctx, MemoryBackend};
use crate::channel::Message;
use crate::coordinate::{Claim, ClaimScope};
use crate::edge::{Edge, EdgeCorpus};
use crate::index::IndexRow;
use crate::query::Query;
use crate::store::{Error, Store};

/// A [`MemoryBackend`] backed by a local markdown+SQLite [`Store`].
///
/// The port is `Send + Sync`, but a [`Store`] wraps a single SQLite connection and is not `Sync`, so
/// access is serialized behind a [`Mutex`]. Local backends are single-writer anyway, so the lock is
/// never contended in practice — it exists only to satisfy the port's threading contract.
pub struct LocalBackend {
    store: Mutex<Store>,
}

impl LocalBackend {
    /// Open the local store rooted at `path` and wrap it as a backend.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, BackendError> {
        Ok(Self {
            store: Mutex::new(Store::open(path)?),
        })
    }

    /// Borrow the wrapped store. A poisoned lock means a prior call panicked mid-write; surface it
    /// as a backend error rather than propagating the panic.
    fn store(&self) -> Result<MutexGuard<'_, Store>, BackendError> {
        self.store
            .lock()
            .map_err(|_| BackendError::Backend("store lock poisoned".into()))
    }
}

/// The author for ledger and coordination entries: `ctx.actor`, or `"local"` when unset.
fn actor(ctx: &Ctx) -> &str {
    ctx.actor.as_deref().unwrap_or("local")
}

/// Map a store error onto the backend-agnostic port error. Structural failures (`Invalid`,
/// `Conflict`, `NotFound`) keep their meaning; the driver-level variants collapse to a short, safe
/// category message so no raw I/O, database, or document text can leak through the port.
impl From<Error> for BackendError {
    fn from(e: Error) -> Self {
        match e {
            Error::Invalid(violations) => BackendError::Invalid(violations),
            Error::Conflict(msg) => BackendError::Conflict(msg),
            Error::NotFound(id) => BackendError::NotFound(id),
            Error::Io(_) => BackendError::Backend("storage i/o error".into()),
            Error::Db(_) => BackendError::Backend("storage index error".into()),
            Error::Parse(_) => BackendError::Backend("stored document parse error".into()),
            Error::Unsigned => BackendError::Backend("signing key unavailable".into()),
        }
    }
}

impl MemoryBackend for LocalBackend {
    fn write(&self, _ctx: &Ctx, memory: &mut Memory) -> Result<String, BackendError> {
        Ok(self.store()?.write(memory)?)
    }

    fn update(
        &self,
        _ctx: &Ctx,
        id: &str,
        topic: Option<String>,
        body: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<bool, BackendError> {
        Ok(self.store()?.update(id, topic, body, tags)?)
    }

    fn supersede(
        &self,
        _ctx: &Ctx,
        old_id: &str,
        new: &mut Memory,
    ) -> Result<String, BackendError> {
        Ok(self.store()?.supersede(old_id, new)?)
    }

    fn delete(&self, _ctx: &Ctx, id: &str) -> Result<bool, BackendError> {
        Ok(self.store()?.delete(id)?)
    }

    fn read(&self, _ctx: &Ctx, id: &str) -> Result<Option<Memory>, BackendError> {
        Ok(self.store()?.read(id)?)
    }

    fn list(&self, _ctx: &Ctx) -> Result<Vec<IndexRow>, BackendError> {
        Ok(self.store()?.list()?)
    }

    fn query(&self, _ctx: &Ctx, q: &Query) -> Result<Vec<Memory>, BackendError> {
        Ok(self.store()?.query(q)?)
    }

    fn revision(&self, _ctx: &Ctx) -> Result<String, BackendError> {
        Ok(self.store()?.revision()?)
    }

    fn areas(&self, _ctx: &Ctx) -> Result<Vec<(String, usize)>, BackendError> {
        Ok(self.store()?.areas()?)
    }

    fn keyword_candidates(
        &self,
        _ctx: &Ctx,
        text: &str,
        limit: usize,
    ) -> Result<Vec<IndexRow>, BackendError> {
        Ok(self.store()?.keyword_candidates(text, limit)?)
    }

    fn vector_candidates(
        &self,
        _ctx: &Ctx,
        seed: &[f32],
        top_k: usize,
        filter: &Query,
    ) -> Result<Vec<(String, f32)>, BackendError> {
        Ok(self.store()?.vector_candidates(seed, top_k, filter)?)
    }

    fn neighbors(&self, _ctx: &Ctx, ids: &[String]) -> Result<Vec<Edge>, BackendError> {
        let store = self.store()?;
        let rows = store.list()?;
        let vectors: HashMap<String, Vec<f32>> = store.vectors()?.into_iter().collect();
        let corpus = EdgeCorpus::new(&rows, vectors);
        let mut out = Vec::new();
        for id in ids {
            out.extend(corpus.edges_from(id));
        }
        Ok(out)
    }

    fn log_event(&self, ctx: &Ctx, kind: &str, summary: &str) -> Result<(), BackendError> {
        Ok(self.store()?.log_event(kind, actor(ctx), summary)?)
    }

    fn log_retrieval(&self, ctx: &Ctx, query: &str, ids: &[String]) -> Result<(), BackendError> {
        Ok(self.store()?.log_retrieval(actor(ctx), query, ids)?)
    }

    fn recall_counts(&self, _ctx: &Ctx) -> Result<HashMap<String, u32>, BackendError> {
        Ok(self.store()?.recall_counts()?)
    }

    fn active_claims(&self, _ctx: &Ctx) -> Result<Vec<Claim>, BackendError> {
        Ok(self.store()?.active_claims()?)
    }

    fn put_claim(
        &self,
        ctx: &Ctx,
        session_id: &str,
        scope: ClaimScope,
        intent: &str,
        ttl_secs: i64,
    ) -> Result<Claim, BackendError> {
        Ok(self
            .store()?
            .claim(session_id, actor(ctx), scope, intent, ttl_secs)?)
    }

    fn room_messages(&self, _ctx: &Ctx, thread: &str) -> Result<Vec<Message>, BackendError> {
        Ok(self.store()?.thread(thread)?)
    }

    fn post_message(
        &self,
        ctx: &Ctx,
        to: &str,
        thread: Option<&str>,
        role: &str,
        body: &str,
    ) -> Result<String, BackendError> {
        Ok(self
            .store()?
            .post_message(actor(ctx), to, thread, role, body)?)
    }
}

#[cfg(test)]
mod tests {
    use marrow_memdocs::{Frontmatter, Memory, MemoryKind, Provenance, Scope, Status};

    use crate::backend::{Ctx, MemoryBackend};
    use crate::backend_local::LocalBackend;
    use crate::store::Store;

    // The engine holds a `Box<dyn MemoryBackend>`; this proves `LocalBackend` satisfies the port's
    // object-safety and `Send + Sync` contract.
    fn _assert_object_safe(_: &dyn MemoryBackend) {}

    fn memory(kind: MemoryKind, topic: &str, body: &str) -> Memory {
        Memory {
            frontmatter: Frontmatter {
                id: String::new(),
                kind,
                status: Status::Active,
                topic: Some(topic.into()),
                area: None,
                scope: Scope {
                    project_id: "demo".into(),
                },
                refs: vec![],
                code_anchors: vec![],
                confidence: 1.0,
                decay: None,
                provenance: Provenance {
                    written_by: "test".into(),
                    model: None,
                    session_id: None,
                    sources: vec![],
                },
                supersedes: vec![],
                tags: vec![],
                created_at: String::new(),
                updated_at: String::new(),
                hmac: None,
            },
            body: body.into(),
        }
    }

    #[test]
    fn local_backend_write_read_matches_store() {
        let dir = tempfile::tempdir().unwrap();
        let be = LocalBackend::open(dir.path()).unwrap();
        _assert_object_safe(&be);
        let ctx = Ctx::default();
        let mut m = memory(MemoryKind::Decision, "auth", "Use JWT.");
        let id = be.write(&ctx, &mut m).unwrap();
        let got = be.read(&ctx, &id).unwrap().unwrap();
        assert_eq!(got.body.trim(), "Use JWT.");
    }

    #[test]
    fn local_backend_list_matches_store() {
        let dir = tempfile::tempdir().unwrap();
        let be = LocalBackend::open(dir.path()).unwrap();
        let ctx = Ctx::default();

        let corpus = [
            memory(MemoryKind::Decision, "auth", "Use JWT."),
            memory(MemoryKind::Fact, "billing", "Stripe is the processor."),
            memory(MemoryKind::Fact, "infra", "Postgres backs the index."),
        ];
        for mut m in corpus {
            be.write(&ctx, &mut m).unwrap();
        }

        // A second store opened on the same files reads the same derived index, so its `list()` is
        // the ground truth the backend must reproduce row-for-row.
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(be.list(&ctx).unwrap(), store.list().unwrap());
    }
}
