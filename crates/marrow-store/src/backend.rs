//! The persistence port: the storage surface the engine depends on, so one engine can back both
//! local markdown+SQLite and cloud Postgres. Only persistence lives here — recall ranking,
//! associative-graph traversal, consolidation, and freshness stay *above* the trait, shared across
//! backends. The `keyword_candidates`/`vector_candidates`/`neighbors` split is the seam that lets a
//! local backend brute-force cosine while a Postgres backend does pgvector ANN.

use std::collections::HashMap;
use std::fmt;

use marrow_memdocs::{Memory, Violation};

use crate::channel::Message;
use crate::coordinate::{Claim, ClaimScope};
use crate::edge::Edge;
use crate::index::IndexRow;
use crate::query::Query;

/// Tenant and actor context threaded through every backend call. A local backend can ignore it; a
/// Postgres backend uses it to set the RLS tenant GUCs and to attribute writes. Where a method
/// needs an author (ledger entries, claims, messages), it is sourced from [`Ctx::actor`].
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// The owning organization (tenant), when hosted.
    pub org: Option<String>,
    /// The project the operation is scoped to.
    pub project: Option<String>,
    /// Who is acting — sourced into ledger and coordination entries that need an author.
    pub actor: Option<String>,
}

/// Errors a backend can return. Deliberately backend-agnostic: it names failure *kinds*, not the
/// SQLite/Postgres machinery behind them.
#[derive(Debug)]
pub enum BackendError {
    /// A memory failed schema validation; carries every violation.
    Invalid(Vec<Violation>),
    /// A uniqueness or lifecycle constraint was violated.
    Conflict(String),
    /// The requested memory does not exist.
    NotFound(String),
    /// The backend rejected the operation (I/O, database, network, or auth).
    Backend(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Invalid(vs) => {
                let joined = vs
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "invalid memory: {joined}")
            }
            BackendError::Conflict(m) => write!(f, "conflict: {m}"),
            BackendError::NotFound(id) => write!(f, "memory not found: {id}"),
            BackendError::Backend(m) => write!(f, "backend error: {m}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// The persistence port. Object-safe and `Send + Sync` so the engine can hold a
/// `Box<dyn MemoryBackend>` and drive one shared code path across local and cloud backends.
pub trait MemoryBackend: Send + Sync {
    // --- memory lifecycle ---

    /// Validate, persist, and index a memory. Returns its id.
    fn write(&self, ctx: &Ctx, memory: &mut Memory) -> Result<String, BackendError>;

    /// Edit a memory in place (topic/body/tags), keeping its id and lineage. False if absent.
    fn update(
        &self,
        ctx: &Ctx,
        id: &str,
        topic: Option<String>,
        body: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<bool, BackendError>;

    /// Mark `old_id` superseded and write `new` in its place, recording the lineage. Returns the
    /// new id.
    fn supersede(&self, ctx: &Ctx, old_id: &str, new: &mut Memory) -> Result<String, BackendError>;

    /// Delete a memory outright. False if there is no such memory.
    fn delete(&self, ctx: &Ctx, id: &str) -> Result<bool, BackendError>;

    /// Read a memory by id, if present.
    fn read(&self, ctx: &Ctx, id: &str) -> Result<Option<Memory>, BackendError>;

    /// Lightweight listing of all indexed memories (most recent first).
    fn list(&self, ctx: &Ctx) -> Result<Vec<IndexRow>, BackendError>;

    /// Structured query, loading full memories under the query's optional token budget.
    fn query(&self, ctx: &Ctx, q: &Query) -> Result<Vec<Memory>, BackendError>;

    /// A cheap fingerprint of the store's state: enough to tell whether anything changed.
    fn revision(&self, ctx: &Ctx) -> Result<String, BackendError>;

    /// The project's feature areas with their memory counts, busiest first.
    fn areas(&self, ctx: &Ctx) -> Result<Vec<(String, usize)>, BackendError>;

    // --- retrieval primitives (the backend chooses brute-force vs ANN) ---

    /// Keyword-lane candidates for `text`, capped at `limit`. Raw candidates — fusion and ranking
    /// happen above the trait.
    fn keyword_candidates(
        &self,
        ctx: &Ctx,
        text: &str,
        limit: usize,
    ) -> Result<Vec<IndexRow>, BackendError>;

    /// The `top_k` memories nearest `seed` by embedding similarity, as `(id, score)`, restricted to
    /// rows matching `filter`. The filter is applied BEFORE the top-k (a filtered pgvector ANN on a
    /// Postgres backend; a filtered cosine scan locally), so a selective filter never loses its
    /// matching-but-lower-cosine rows to a global top-k.
    fn vector_candidates(
        &self,
        ctx: &Ctx,
        seed: &[f32],
        top_k: usize,
        filter: &Query,
    ) -> Result<Vec<(String, f32)>, BackendError>;

    /// The directly related edges for each of `ids` (refs, shared topic/tag, semantic neighbours) —
    /// the adjacency the engine's associative recall walks.
    fn neighbors(&self, ctx: &Ctx, ids: &[String]) -> Result<Vec<Edge>, BackendError>;

    // --- ledger / audit ---

    /// Record an arbitrary agent-authored episodic event (author taken from `ctx.actor`).
    fn log_event(&self, ctx: &Ctx, kind: &str, summary: &str) -> Result<(), BackendError>;

    /// Record a retrieval: which memory ids were recalled for a query. Makes an answer traceable
    /// and teaches the brain which memories are worth reaching for.
    fn log_retrieval(&self, ctx: &Ctx, query: &str, ids: &[String]) -> Result<(), BackendError>;

    /// How many times each memory has been recalled.
    fn recall_counts(&self, ctx: &Ctx) -> Result<HashMap<String, u32>, BackendError>;

    // --- coordination ---

    /// All work-claims that are still active: registered, not released, renewed-or-not-expired.
    fn active_claims(&self, ctx: &Ctx) -> Result<Vec<Claim>, BackendError>;

    /// Register an advisory work-claim (holder taken from `ctx.actor`). Returns the created claim.
    fn put_claim(
        &self,
        ctx: &Ctx,
        session_id: &str,
        scope: ClaimScope,
        intent: &str,
        ttl_secs: i64,
    ) -> Result<Claim, BackendError>;

    /// Every message in a thread/room, oldest first.
    fn room_messages(&self, ctx: &Ctx, thread: &str) -> Result<Vec<Message>, BackendError>;

    /// Post a message (sender taken from `ctx.actor`). Pass `thread` to reply within an existing
    /// room, or `None` to start one. Returns the thread id.
    fn post_message(
        &self,
        ctx: &Ctx,
        to: &str,
        thread: Option<&str>,
        role: &str,
        body: &str,
    ) -> Result<String, BackendError>;
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use marrow_memdocs::Memory;

    use crate::backend::{BackendError, Ctx, MemoryBackend};
    use crate::channel::Message;
    use crate::coordinate::{Claim, ClaimScope};
    use crate::edge::Edge;
    use crate::index::IndexRow;
    use crate::query::Query;

    // Object safety is load-bearing: the engine holds a `Box<dyn MemoryBackend>` and shares one
    // code path across backends. If any method takes `self` by value, is generic, or returns
    // `impl Trait`, this stops compiling.
    fn _assert_object_safe(_: &dyn MemoryBackend) {}

    // A do-nothing backend proves the port is total and implementable in isolation. When the trait
    // grows a method, this stub stops compiling until it is covered.
    struct StubBackend;

    impl MemoryBackend for StubBackend {
        fn write(&self, _ctx: &Ctx, _memory: &mut Memory) -> Result<String, BackendError> {
            Ok(String::new())
        }
        fn update(
            &self,
            _ctx: &Ctx,
            _id: &str,
            _topic: Option<String>,
            _body: Option<String>,
            _tags: Option<Vec<String>>,
        ) -> Result<bool, BackendError> {
            Ok(false)
        }
        fn supersede(
            &self,
            _ctx: &Ctx,
            _old_id: &str,
            _new: &mut Memory,
        ) -> Result<String, BackendError> {
            Ok(String::new())
        }
        fn delete(&self, _ctx: &Ctx, _id: &str) -> Result<bool, BackendError> {
            Ok(false)
        }
        fn read(&self, _ctx: &Ctx, _id: &str) -> Result<Option<Memory>, BackendError> {
            Ok(None)
        }
        fn list(&self, _ctx: &Ctx) -> Result<Vec<IndexRow>, BackendError> {
            Ok(Vec::new())
        }
        fn query(&self, _ctx: &Ctx, _q: &Query) -> Result<Vec<Memory>, BackendError> {
            Ok(Vec::new())
        }
        fn revision(&self, _ctx: &Ctx) -> Result<String, BackendError> {
            Ok(String::new())
        }
        fn areas(&self, _ctx: &Ctx) -> Result<Vec<(String, usize)>, BackendError> {
            Ok(Vec::new())
        }
        fn keyword_candidates(
            &self,
            _ctx: &Ctx,
            _text: &str,
            _limit: usize,
        ) -> Result<Vec<IndexRow>, BackendError> {
            Ok(Vec::new())
        }
        fn vector_candidates(
            &self,
            _ctx: &Ctx,
            _seed: &[f32],
            _top_k: usize,
            _filter: &Query,
        ) -> Result<Vec<(String, f32)>, BackendError> {
            Ok(Vec::new())
        }
        fn neighbors(&self, _ctx: &Ctx, _ids: &[String]) -> Result<Vec<Edge>, BackendError> {
            Ok(Vec::new())
        }
        fn log_event(&self, _ctx: &Ctx, _kind: &str, _summary: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn log_retrieval(
            &self,
            _ctx: &Ctx,
            _query: &str,
            _ids: &[String],
        ) -> Result<(), BackendError> {
            Ok(())
        }
        fn recall_counts(&self, _ctx: &Ctx) -> Result<HashMap<String, u32>, BackendError> {
            Ok(HashMap::new())
        }
        fn active_claims(&self, _ctx: &Ctx) -> Result<Vec<Claim>, BackendError> {
            Ok(Vec::new())
        }
        fn put_claim(
            &self,
            ctx: &Ctx,
            session_id: &str,
            scope: ClaimScope,
            intent: &str,
            ttl_secs: i64,
        ) -> Result<Claim, BackendError> {
            Ok(Claim {
                id: String::new(),
                session_id: session_id.to_string(),
                actor: ctx.actor.clone().unwrap_or_default(),
                scope,
                intent: intent.to_string(),
                created_at: String::new(),
                expires_at: String::new(),
                ttl_secs,
            })
        }
        fn room_messages(&self, _ctx: &Ctx, _thread: &str) -> Result<Vec<Message>, BackendError> {
            Ok(Vec::new())
        }
        fn post_message(
            &self,
            _ctx: &Ctx,
            _to: &str,
            _thread: Option<&str>,
            _role: &str,
            _body: &str,
        ) -> Result<String, BackendError> {
            Ok(String::new())
        }
    }

    #[test]
    fn stub_backend_is_object_safe_and_total() {
        let backend = StubBackend;
        _assert_object_safe(&backend);
        let ctx = Ctx::default();
        assert!(backend.list(&ctx).unwrap().is_empty());
        assert!(backend.read(&ctx, "missing").unwrap().is_none());
    }
}
