# Marrow — Evidence

Six measurements: an end-to-end A/B on a real agent, a research spike over a Python corpus, and four
component benchmarks that reproduce from a clean checkout with no network or model download.

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

Warm was flat at approximately 37,800 tokens in these runs, while cold ranged from 98k to 170k. A
warm session recalled a fixed briefing; a cold one explored the repository. Larger repositories may
increase that gap, but this experiment did not test repository-size scaling.

Honest scope: this is an *amortized* result — the one-time cost of distilling memory into Marrow is
not charged per query (that is the point of a shared brain). It is question- and codebase-dependent
(broad "orient me" tasks benefit most) and, like any LLM measurement, noisy — hence multiple runs.
It compares a populated brain against cold exploration, which is Marrow's real use case rather than
a controlled micro-benchmark.

Method: `claude -p "<prompt>" --output-format json` against a stripped vs a populated copy of the
repo, summing `usage` tokens and reading `duration_ms`.

## 1. Research StaleEval — is the hybrid worth building?

A code repository is seeded with memories anchored to specific symbols; the code is then
mutated (reformat, rename, move, change-logic, change-signature, delete) and each anchoring
strategy is scored on whether it tells a genuine change from a harmless one.

**Result (hybrid structural + relocation):** ~**1.0% false-positive rate** at ~**98.4% recall**.
This result comes from synthetic Python mutations over one pinned corpus. It motivated the hybrid
design; it is not a measurement of the shipping Rust parser.

Reproduce: `cd spike/staleness && ./fetch_corpus.sh && python3 -m staleness_spike.run`.

## 2. FreshEval — does the shipping Rust engine handle its documented cases?

Seven deterministic cases run directly against `marrow-core`: unchanged code, reformatting, local
rename, logic change, signature change, deletion, and cross-file relocation.

| Metric | Value |
|---|---|
| Correct classifications | 7 / 7 |
| False positives | 0 |
| False negatives | 0 |
| Relocations preserved | 1 / 1 |

This verifies the documented Rust cases, not production precision across real repository histories.

Reproduce: `cargo run -p marrow-bench`.

## 3. ConsolEval — is conservative duplicate detection correct?

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

Honest scope: ConsolEval uses six small labeled cases and a deterministic lexical embedder. It
tests clustering and survivor selection under those inputs, not every embedding model or genuine
contradiction. The local distiller de-duplicates lines; it does not resolve semantic conflicts.

Reproduce: `cargo run -p marrow-bench`.

## 4. TokenEval — does it keep retrieval cheap?

A broad query over a 40-memory corpus is run with and without a token budget.

**Result:** full result ≈ **2600 tokens**; budgeted to 500 ≈ **455 tokens** — an **~82% reduction**.
Memory recall stays useful while the context window stays focused.

Reproduce: `cargo run -p marrow-bench`.

## 5. CoordEval — do advisory claims distinguish conflicts from safe work?

Four overlapping file scopes and three unrelated scopes are checked through the shipping claim
store, followed by release.

| Metric | Value |
|---|---|
| Conflicts detected | 4 / 4 |
| Safe scopes allowed | 3 / 3 |
| False blocks | 0 |
| Claims released | yes |

This verifies the coordination primitive. Claude hooks remain best-effort and fail open; this is not
a claim that every real multi-agent collision is prevented.

Reproduce: `cargo run -p marrow-bench`.

---

These measurements support narrower claims: Marrow budgets retrieval, conservatively consolidates
the labeled duplicates, handles its documented Rust freshness cases, and detects overlapping claim
scopes. The A/B suggests meaningful orientation savings on one repository. Broader user outcomes
need the longitudinal study in [OUTCOME-STUDY.md](OUTCOME-STUDY.md).
