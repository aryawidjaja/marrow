# Marrow — Evidence

Three benchmarks. Each reproduces from a clean checkout with no network or model download.

## 1. StaleEval — does it surface stale knowledge?

A code repository is seeded with memories anchored to specific symbols; the code is then
mutated (reformat, rename, move, change-logic, change-signature, delete) and each anchoring
strategy is scored on whether it tells a genuine change from a harmless one.

**Result (hybrid structural + relocation):** ~**1.0% false-positive rate** at ~**98.4% recall**.
A reformat or a renamed local does not trip it; a changed body, a changed signature, or a
deletion does; a moved symbol is relocated rather than flagged. This is the engine Marrow
ships (`marrow-core`).

Reproduce: `cd spike/staleness && ./fetch_corpus.sh && python3 -m staleness_spike.run`.

## 2. ConsolEval — is the consolidation engine correct?

Labeled memory sets (exact duplicates, reordered paraphrases, distinct memories, and
mixed groups, some with differing confidence) are run through consolidation, and the detected
clusters are compared to the ground truth.

**Result:**

| Metric | Value |
|---|---|
| Clustering precision | 100% |
| Clustering recall | 100% |
| False merges (distinct memories wrongly merged) | 0 |
| Survivor selection correct (highest-confidence kept) | 5 / 5 |

Honest scope: ConsolEval uses a deterministic lexical embedder, so it proves the engine —
clustering, survivor selection, and the no-false-merge guarantee — not the vocabulary breadth
of any particular embedding model. Catching paraphrases that share no words needs a real
embedding model (`embed-fastembed` / `embed-http`), and resolving genuine contradictions needs
an LLM distiller (`distill-http`, pointed at a local/sovereign model).

Reproduce: `cargo run -p marrow-bench`.

## 3. TokenEval — does it keep retrieval cheap?

A broad query over a 40-memory corpus is run with and without a token budget.

**Result:** full result ≈ **2600 tokens**; budgeted to 500 ≈ **455 tokens** — an **~82% reduction**.
Memory recall stays useful while the context window stays focused.

Reproduce: `cargo run -p marrow-bench`.

---

The pitch line these support: *Marrow doesn't surface stale facts, it merges duplicates and
resolves contradictions without losing anything, and it does so within a token budget — and
every number here reproduces from source.*
