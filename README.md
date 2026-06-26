# Marrow 🦴

*Persistent, shared memory so your AI agents stop forgetting — and a hive mind so a swarm of them works as one.*

[![Release](https://img.shields.io/github/v/release/aryawidjaja/marrow?color=2ea44f&label=release)](https://github.com/aryawidjaja/marrow/releases/latest)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Stars](https://img.shields.io/github/stars/aryawidjaja/marrow?style=flat&logo=github&color=ffd33d)](https://github.com/aryawidjaja/marrow/stargazers)

[![MCP](https://img.shields.io/badge/MCP-compatible-8A2BE2?logo=modelcontextprotocol&logoColor=white)](https://modelcontextprotocol.io)
[![Claude Code](https://img.shields.io/badge/Claude%20Code-compatible-D97757?logo=claude&logoColor=white)](https://www.anthropic.com/claude-code)
[![Cursor](https://img.shields.io/badge/Cursor-compatible-000000?logo=cursor&logoColor=white)](https://cursor.com)
[![Codex](https://img.shields.io/badge/Codex-compatible-412991)](https://openai.com/codex)

## What is Marrow

Marrow is a memory for your AI coding agents.

Normally every new session starts from zero. The agent re-reads your codebase, repeats decisions you
already made, and forgets everything the moment its context window fills up. Run several agents at
once and it gets worse — blind to each other, they collide and undo each other's work.

Marrow gives them one shared memory that **persists** across sessions, **stays current** as your code
changes, and lets **many agents work together** without stepping on each other. The memory is plain
markdown files you can read, edit, and commit to git — not a black box. And unlike a `CLAUDE.md` you
maintain by hand, Marrow keeps itself fresh and merges duplicates on its own.

And it pays for itself in tokens. Ask the same "understand this codebase" question through Claude Code
against this repo — once cold, once warm with Marrow — and the warm run uses about **72% fewer
tokens** and finishes about **57% faster**, because it recalls a small distilled briefing instead of
re-reading every file:

<img src="assets/benchmark.png" width="720" alt="Marrow benchmarks: 72% fewer tokens, 57% faster, 25% cheaper; 98% stale-knowledge recall, 100% consolidation precision, 82% smaller retrieval payload">

| Metric | Cold (reads files) | Warm (Marrow) | Saved |
|---|---|---|---|
| Tokens | ~134k | ~38k | **~72%** |
| Time | ~26s | ~11s | **~57%** |
| Cost | ~$0.21 | ~$0.16 | ~25% |

The tell is the variance: **warm stays flat at ~38k tokens every run, while cold swings from 98k to
170k.** A warm session recalls a fixed, distilled briefing; a cold one re-reads the codebase — so the
gap *widens* on larger projects. Engine benchmarks (staleness ~1% false-positive at ~98% recall,
consolidation 100% clustering precision / 0 false merges, ~82% retrieval-budget token cut).

## Quickstart

Wire it up, then let your agents fill the memory.

**1. Install** (Homebrew, macOS / Linux — other options under [Install](#install)):
```bash
brew install aryawidjaja/marrow/marrow
```

**2. Set it up** from your project's root folder:
```bash
marrow setup
```
This registers Marrow with your agent, installs the automatic hooks, and adds a short guidance note.
Want every repo wired at once? Run `marrow setup --global` instead to install the hooks user-wide.

**3. Restart your agent.** Reading is now automatic: every session starts warm — it loads what's
already in the brain instead of re-reading everything — and claims the files it touches so parallel
sessions don't collide.

**4. Let it capture.** Start a fresh session and the agent uses Marrow on its own — saving decisions,
context, and checkpoints as it works, so the next session picks up where this one left off. Already
deep in a session from before you ran setup? Run **`/marrow-save`** in it once to capture everything
it knows into the brain.

The memory lives under `.marrow/` in your project.

## Install

Homebrew (macOS / Linux):
```bash
brew install aryawidjaja/marrow/marrow
```
Prebuilt binaries, no Rust needed:
```bash
curl -fsSL https://raw.githubusercontent.com/aryawidjaja/marrow/main/install.sh | sh
```
From source with Rust:
```bash
cargo install --git https://github.com/aryawidjaja/marrow marrow-cli marrow-mcp
```
Each puts `marrow` and `marrow-mcp` on your PATH. Add `marrow-web` for the dashboard.

## Bringing in an existing project

A fresh project's memory starts empty. To seed it from the docs you already have, the first warm
start nudges your agent to run `marrow ingest` — it lists your README and `docs/` files and distills
them into memory. After that, every session starts informed.

Any time you want to capture the session you're in, run **`/marrow-save`** and the agent writes what
you decided to the shared brain.

## Using Cursor, Codex, or tools without the hooks

The automatic hooks (warm start, collision guard, activity trail) are specific to Claude Code. With
any other MCP agent you still get the full memory toolset — capture just becomes a tool the agent
calls instead of a background reflex.

Register the MCP server for every Claude Code project:
```bash
claude mcp add marrow -s user -- marrow-mcp --root .
```
For a single project instead, add the same server to `.mcp.json` (Claude Code), `.cursor/mcp.json`
(Cursor), or your Codex TOML.

Either way your agent gets the `mem_*` tools — `mem_write` / `mem_recall` / `mem_search` for memory,
and `mem_bootstrap` / `mem_claim` / `mem_activity` for the shared brain — plus the `save` prompt.

## Optional: smarter (semantic) search

By default, search is keyword-based (FTS5) — small, instant, fully offline, and enough for most
people. If you want **semantic** recall — finding a note about "JWT" when you search "login
security" — install a build with an embedding model and turn it on.

Homebrew, semantic build (multilingual including Arabic; downloads a small model on first use):
```bash
brew install aryawidjaja/marrow/marrow-semantic
```
```bash
marrow embed fastembed
```

Or build the same local model from source with cargo:
```bash
cargo install --git https://github.com/aryawidjaja/marrow marrow-cli marrow-mcp --features embed-fastembed
```
```bash
marrow embed fastembed
```

Or point at any OpenAI-compatible endpoint instead of a local model:
```bash
cargo install --git https://github.com/aryawidjaja/marrow marrow-cli marrow-mcp --features embed-http
```
```bash
marrow embed http --url https://your-endpoint/v1/embeddings
```

The Homebrew and curl binaries are keyword-only, so use a build above for semantic search.
`marrow status` shows the active mode; `marrow embed none` switches back to keyword.

## Optional: one brain across projects or a team

Point your sessions at a shared store instead of the local project — use `--root ~/marrow-shared` in
place of `.`. Memories are scoped per project, so an agent can recall across all of them when it
wants to. This shared/team brain is the direction the served and enterprise editions build on.

## CLI

Most of the time your agent drives Marrow for you. You can also use it directly from the terminal.

Save a memory:
```bash
marrow add --kind decision --topic auth "We use short-lived JWTs."
```
Search it back — `--weight` runs from 0 (keyword) to 1 (semantic):
```bash
marrow search "token expiry" --weight 1
```
List notes whose underlying code has drifted:
```bash
marrow list-stale --repo .
```
Merge duplicates and retire expired notes:
```bash
marrow consolidate --repo . --apply
```
Verify the audit ledger is untampered:
```bash
marrow audit
```
Open the local dashboard:
```bash
marrow-serve --root . --port 8088
```

That `marrow add` doesn't write to an opaque database — it saves a plain markdown file under
`.marrow/memory/` that you (and git) can read and edit by hand. The YAML frontmatter is the
metadata; the text below it is the memory:

```markdown
---
type: decision
topic: auth
confidence: 1.0
---
We use short-lived JWTs for sessions.
```

(That's `.marrow/memory/decision/<id>.md` — the SQLite index is just a rebuildable cache over
these files.)

## What it does

- **Staleness detection** — a memory can cite a code symbol; Marrow fingerprints it and flags the
  note the moment the symbol's behavior or signature changes, while ignoring reformatting and
  renames. If the symbol just moved, it relocates the citation instead of crying stale.
- **Consolidation** — clusters related memories by meaning, merges duplicates, resolves
  contradictions, and retires expired notes, preserving lineage.
- **Hive mind** — many agent sessions work as one swarm over one shared brain: each joins *warm*
  (`bootstrap` — it already knows what others did), `claim`s its work so two never collide, and reads
  a live activity trail each turn — the swarm's pheromone trail — so every agent senses what the
  others are doing. Unlike a black-box hive, it's fully **auditable**: every signal is in the ledger.
  Vendor-neutral over MCP.
- **Audit & provenance** — every write, supersede, and recall lands in an append-only, hash-chained
  ledger; `marrow audit` proves it untampered, and any answer traces back to its sources.
- **Typed & validated** — five schemas (`fact`, `decision`, `entity`, `session`, `skill`); bad
  writes are rejected with reasons; lifecycle via supersede, confidence, and decay.
- **Runs anywhere** — a single offline binary; nothing leaves your machine.

## How it works

- **Markdown is the source of truth.** The SQLite index under `.marrow/.index` is a disposable
  cache — `marrow doctor` rebuilds it from the files. Writes are atomic (temp file + rename).
- **Search** is keyword (FTS5) by default; semantic (vector embeddings, fused with keyword via
  reciprocal rank fusion, tuned by `--weight`) is an opt-in — see [Optional: smarter (semantic) search](#optional-smarter-semantic-search).
- **Staleness** records a structural fingerprint of the cited symbol plus a normalized copy for
  relocation; a note is stale only when the code is genuinely gone or changed. Rust today, via
  tree-sitter; other languages by adding grammars.
- **Consolidation** judges each cluster with a pluggable distiller (merge / resolve / keep) —
  deterministic by default, or pointed at a local or sovereign-hosted LLM, so nothing leaves your
  infrastructure.
- **It's a library.** `marrow-store` is a normal Rust crate; the CLI, MCP server, and dashboard are
  thin layers over it.

## Anthropic memory-tool backend

`python/marrow-anthropic` implements Anthropic's memory tool (`memory_20250818`) with strict path
confinement — see its [README](python/marrow-anthropic/README.md).

## The name

Marrow is the essential core a body grows from — and, biologically, where the immune system's
memory begins: the quiet, foundational layer an agent's knowledge is built on and remembered from.

## License

Dual-licensed: the engine (`crates/`) is **AGPL-3.0-only**; the embeddable Python backend
(`python/marrow-anthropic`) is **Apache-2.0**. Using Marrow from your agent over MCP or the CLI is a
separate process, not a derivative work. A commercial license is available — see
[COMMERCIAL.md](COMMERCIAL.md).
