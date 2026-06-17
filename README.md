# Marrow

Marrow is a memory store for AI agents that keeps everything in plain markdown files while
giving you the things a real database provides: schemas, validation, structured queries,
full-text search, provenance, decay, and — the part most file-based memory gets wrong —
**it tells you when a memory has gone out of date.**

Agents accumulate notes: decisions, facts about a codebase, things they learned last
session. Stored as loose markdown, those notes rot quietly. You rename a function and the
note that describes it keeps claiming the old behavior. Marrow anchors a memory to the code
it talks about and flags it the moment that code changes underneath it.

The files stay human-readable and git-friendly. The database lives beside them as a
rebuildable index, never as the source of truth.

## Why it exists

File-based memory (`CLAUDE.md`, `.cursorrules`, and friends) won out for agents because it's
transparent, version-controllable, and needs no infrastructure. But in practice it has real
gaps:

- **It goes stale silently.** Nothing warns you when a note no longer matches the code.
- **It can't be queried.** Loading a whole file into context to use one paragraph wastes
  the model's attention.
- **Nothing is validated.** An agent can write a malformed or contradictory note and poison
  later sessions.
- **There's no provenance or lifecycle.** No record of who wrote what, when, or whether a
  decision was later reversed.

Marrow keeps the markdown-first approach and closes those gaps.

## What you get

- **Staleness detection.** A memory can cite a code symbol. Marrow stores a structural
  fingerprint of that symbol and checks it against the live code. Reformatting or renaming a
  local variable does not trip it; changing the logic, the signature, or deleting the symbol
  does. If the symbol simply moved to another file, Marrow finds it and relocates the
  citation instead of crying stale.
- **Typed documents.** Every memory is a markdown file with YAML frontmatter conforming to
  one of five base schemas: `fact`, `decision`, `entity`, `session`, `skill`.
- **Validated writes.** Bad writes are rejected with the specific reasons, so an agent can
  fix and retry. At most one active decision is allowed per topic per project.
- **Hybrid search.** Structured queries plus a single search that fuses keyword matching
  (SQLite FTS5) with semantic vector similarity, tunable from pure keyword to pure semantic.
  An optional token budget caps how much a search returns.
- **Lifecycle and decay.** Memories can supersede one another, carry a confidence score, and
  expire or decay over time.
- **Provenance and integrity.** Each memory records who wrote it; optional HMAC signing makes
  tampering detectable.
- **Tamper-evident audit trail.** Every write, supersede, and agent observation is recorded
  in an append-only, hash-chained ledger. Edit any past entry and `marrow audit` reports the
  break. Nothing is ever deleted from it.
- **Consolidation that learns.** A pass that keeps memory coherent: it clusters related
  memories by *meaning* (embedding similarity), then merges duplicates, resolves
  contradictions, and retires expired notes — distilling rather than dropping, choosing the
  survivor by salience and preserving lineage. Contradiction resolution can run against a
  local or sovereign-hosted LLM (it never has to leave your infrastructure).

## Repository layout

```
crates/
  marrow-core      Code-anchored staleness: structural fingerprint + relocation search
  marrow-memdocs   The memory document format: typed frontmatter, schemas, validation
  marrow-episodic  Append-only, hash-chained event ledger (episodic memory + audit trail)
  marrow-store     Persistence, SQLite/FTS5 index, hybrid search, decay, scope, consolidation
  marrow-cli       The `marrow` command-line tool
  marrow-mcp       A Model Context Protocol server exposing the store to agents
python/
  marrow-anthropic A backend for Anthropic's memory tool (memory_20250818)
```

## Quick start

Build the binaries:

```bash
cargo build --release
# produces target/release/marrow and target/release/marrow-mcp
```

Create a store and add a memory:

```bash
marrow init
marrow add --kind decision --topic auth "We use short-lived JWTs for sessions."
marrow query --kind decision
marrow search JWT                 # hybrid keyword + semantic
marrow search "token expiry" --weight 1   # 0 = keyword only, 1 = semantic only
```

Memories are written as markdown you can read and edit by hand:

```markdown
---
id: 01J9Z3K2Q8WXYZ4ABCD5EFGH6
type: decision
status: active
topic: auth
scope:
  project_id: default
refs: []
confidence: 1.0
provenance:
  written_by: cli
  sources: []
supersedes: []
tags: []
created_at: 2026-06-17T09:00:00Z
updated_at: 2026-06-17T09:00:00Z
---

We use short-lived JWTs for sessions.
```

Check which code-anchored memories have drifted from a repository:

```bash
marrow list-stale --repo .
```

Review the tamper-evident history, or keep memory coherent:

```bash
marrow history                    # every write/supersede/observation
marrow audit                      # verify the hash chain is intact
marrow consolidate --repo .       # report stale, expired, and duplicate memories
marrow consolidate --repo . --apply   # merge duplicates and retire expired
```

If you ever lose or delete the index, rebuild it from the files:

```bash
marrow doctor
```

## Use it from an agent (MCP)

`marrow-mcp` speaks the Model Context Protocol over stdio and exposes the store as tools
(`mem_write`, `mem_read`, `mem_query`, `mem_search`, `mem_supersede`, `mem_list_stale`,
`mem_validate`, `mem_status`). Point an MCP-capable client at it:

```json
{
  "mcpServers": {
    "marrow": {
      "command": "marrow-mcp",
      "args": ["--root", "/path/to/your/project"]
    }
  }
}
```

## Use it as an Anthropic memory-tool backend

`python/marrow-anthropic` implements Anthropic's memory tool (`memory_20250818`) — the six
file operations the model expects — with strict path confinement. See its
[README](python/marrow-anthropic/README.md).

```python
from anthropic import Anthropic
from marrow_anthropic import MarrowMemoryBackend

client = Anthropic()
memory = MarrowMemoryBackend("./.marrow/memories")
client.beta.messages.run_tools(model="claude-opus-4-8", messages=[...], tools=[memory])
```

## How staleness works

When a memory references a code symbol, Marrow records two things about it:

1. A **structural fingerprint** — the symbol's syntax tree with formatting and identifier
   names normalized away. This ignores cosmetic edits (reformatting, renaming a local) but
   changes when the behavior, signature, or shape of the code changes.
2. A **normalized copy of the text**, used to locate the symbol if it moves to another file.

A memory is reported stale only when both checks agree the code is gone or changed. If the
fingerprint no longer matches at the recorded location but the text turns up elsewhere, the
symbol moved — Marrow reports the new location rather than a false alarm. Today this works on
Rust source via tree-sitter; the same approach extends to other languages by adding their
grammars.

## Design notes

- **Markdown is the source of truth.** The SQLite index under `.marrow/.index` is derived
  and disposable; `marrow doctor` rebuilds it from the files.
- **Writes are atomic.** A memory is written to a temporary file and renamed into place, so a
  crash never leaves a half-written document.
- **The store is a library.** `marrow-store` is a normal Rust crate; the CLI and MCP server
  are thin layers over it, and you can embed it directly.

## Search and embeddings

Search is hybrid by default: keyword (FTS5) results and semantic (vector cosine) results are
fused with reciprocal rank fusion, weighted by `--weight`. With no embedding backend
configured it is exactly keyword search, so nothing extra is required to get started.

Embeddings are pluggable via the `[embedding]` section of `.marrow/.marrow.toml`:

- `provider = "hash"` — a built-in, dependency-free lexical embedder (good for tests/demos).
- `provider = "http"` (build with `--features embed-http`) — any OpenAI-compatible embedding
  endpoint; the API key comes from `MARROW_EMBED_API_KEY`.
- `provider = "fastembed"` (build with `--features embed-fastembed`) — a local ONNX model,
  fully offline. The default is multilingual, so non-English text (including Arabic) embeds
  well.

Vectors live in the SQLite index and are rebuilt by `marrow doctor`.

Consolidation uses the same embeddings to find related memories, and a pluggable distiller to
judge each cluster (merge / resolve-conflict / keep). The default is deterministic and offline;
set `[consolidation] distiller = "http"` (build with `--features distill-http`) to point at any
OpenAI-compatible chat endpoint — including a local or sovereign-hosted model — with the key in
`MARROW_DISTILL_API_KEY`.

## Status

Working today: the staleness engine, the document format and validation, the store with its
index, hybrid keyword+semantic search, decay, scope, supersession and integrity, the
append-only audit ledger, the consolidation pass, the CLI, the MCP server, and the Anthropic
memory-tool backend. Tested end to end.

Planned: staleness for more languages, richer consolidation (LLM-assisted distillation), and
concurrent multi-writer support.

## License

MIT.
