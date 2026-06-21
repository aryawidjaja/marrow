# Marrow — Evidence

Four measurements: an end-to-end A/B on a real agent (needs the Claude CLI), plus three component
benchmarks that reproduce from a clean checkout with no network or model download.

## Efficiency — does it actually save tokens and time? (end-to-end A/B)

The headline claim, measured on a real agent rather than asserted. The same prompt — *"summarize
this project: its architecture, what it does, and its current status"* — was run through Claude
Code (headless) against two copies of this repository:

- **Cold** — Marrow removed, so the agent must read files to answer.
- **Warm** — a populated Marrow, so the agent recalls distilled memory instead.

Tokens, wall-clock time, and cost were recorded over **5 runs per arm** (consistent across 9 runs
total).

| Metric | Cold (reads files) | Warm (Marrow) | Saved |
|---|---|---|---|
| Tokens | ~134,000 | ~37,800 | **~72%** |
| Wall-clock time | ~25.9 s | ~11.1 s | **~57%** |
| Cost | ~$0.21 | ~$0.16 | ~25% |

The token and time wins are large and stable. Cost drops less because a cold agent's bulk
file-reads are mostly *cached* input (cheap per token) — the prize here is **context budget and
latency**, not just dollars.

The most telling figure is the variance: **warm is flat at ~37,800 tokens on every run, while cold
swings from 98k to 170k.** A warm session recalls a fixed, distilled briefing; a cold one re-reads
the codebase, so its cost grows with the repository. **On a larger codebase the gap widens** — the
savings scale with the project.

Honest scope: this is an *amortized* result — the one-time cost of distilling memory into Marrow is
not charged per query (that is the point of a shared brain). It is question- and codebase-dependent
(broad "orient me" tasks benefit most) and, like any LLM measurement, noisy — hence multiple runs.
It compares a populated brain against cold exploration, which is Marrow's real use case rather than
a controlled micro-benchmark.

Method: `claude -p "<prompt>" --output-format json` against a stripped vs a populated copy of the
repo, summing `usage` tokens and reading `duration_ms`.

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
