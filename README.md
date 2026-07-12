# Marrow Ⰶ

*A shared brain for your AI coding agents, so they stop forgetting, work as one, and you can **see** everything they know.*

[![Release](https://img.shields.io/github/v/release/aryawidjaja/marrow?color=2ea44f&label=release)](https://github.com/aryawidjaja/marrow/releases/latest)
[![License](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Stars](https://img.shields.io/github/stars/aryawidjaja/marrow?style=flat&logo=github&color=ffd33d)](https://github.com/aryawidjaja/marrow/stargazers)

[![MCP](https://img.shields.io/badge/MCP-compatible-8A2BE2?logo=modelcontextprotocol&logoColor=white)](https://modelcontextprotocol.io)
[![Claude Code](https://img.shields.io/badge/Claude%20Code-compatible-D97757?logo=claude&logoColor=white)](https://www.anthropic.com/claude-code)
[![Cursor](https://img.shields.io/badge/Cursor-compatible-000000?logo=cursor&logoColor=white)](https://cursor.com)
[![Codex](https://img.shields.io/badge/Codex-compatible-412991)](https://openai.com/codex)

## You know Pluribus?

In the show almost everyone joins into one shared mind. Carol is one of the few who stay themselves.
She never gets absorbed, but she still gets to use that hive. Through Zosia she can ask the whole
collective anything, and thousands of minds organize around her to get it done. She stays the
individual. The hive works for her.

That is you and your AI agents. Each one is sharp on its own, but they forget everything between
sessions, repeat decisions you already made, and when you run a few at once they trip over each other.
Marrow gives them one shared memory so they act like a real hive instead of a crowd of strangers:
organized, coordinated, far more useful together. And you stay Carol. The memory lives in your marrow,
plain files on your own machine that you read and own, never taken from you without your say-so.

## The problem

Every new agent session starts from zero. It re-reads your codebase, repeats decisions you already
made, and forgets everything the moment its context fills up. Run several agents at once and it gets
worse: blind to each other, they collide and undo each other's work.

## What Marrow does

Marrow gives your agents **one shared memory** that persists across sessions, stays current as your
code changes, and lets many agents work together without stepping on each other. It's plain markdown
files you can read, edit, and commit, not a black box. A new session starts already knowing what the
others learned.

And it pays for itself in tokens. Ask the same "understand this codebase" question through Claude Code
once cold and once warm with Marrow, and the warm run uses about **72% fewer tokens** and finishes
about **57% faster**, because it recalls a small distilled briefing instead of re-reading every file.

<img src="assets/benchmark.png" width="720" alt="Marrow: 72% fewer tokens, 57% faster, 25% cheaper">

| Metric | Cold (reads files) | Warm (Marrow) | Saved |
|---|---|---|---|
| Tokens | ~134k | ~38k | **~72%** |
| Time | ~26s | ~11s | **~57%** |
| Cost | ~$0.21 | ~$0.16 | ~25% |

The tell is the variance: warm stays flat at ~38k tokens every run while cold swings from 98k to
170k, so the gap *widens* on bigger projects.

## Get started in 3 steps

**1. Install** (macOS / Linux; other options below):
```bash
brew install aryawidjaja/marrow/marrow
```

**2. Set it up** from your project's root:
```bash
marrow setup          # add --global to wire every repo at once
```

**3. Restart your agent.** That's it. Sessions now start warm, capture decisions as they work, and
claim files so parallel agents don't collide. Already mid-session? Run **`/marrow-save`** once to
pour what it knows into the brain.

The memory lives in `.marrow/` in your project.

## See your brain

Marrow isn't a black box, it's a graph you can explore, like a second brain.

```bash
marrow-serve          # opens the dashboard at http://localhost:8088
```

Every memory is a neuron; links connect memories that share a topic, a tag, or **related meaning**
(from embeddings). Drag, zoom, click to read, search, and **add, edit, or delete** memories right
there. Add projects by browsing your folders. It's the whole product, visual and interactive.

## One brain across your projects

By default each project has its own brain. Opt any project into a machine-wide **hive** with one
command, and your agents can recall across all of them:

```bash
cd ~/code/webapp && marrow hub register --name webapp
cd ~/code/api    && marrow hub register --name api

marrow hub recall "how do we do auth"   # searches every project, tagged by project
```

Now an agent working in `api` can ask what `webapp` knows. In the dashboard, the **Hive** tab shows a
central *core* neuron (you) with every project orbiting it, bridged where they share ideas.

## One brain across your devices

Run the Marrow backbone once and point every machine at it. One hive mind, across computers:

```bash
# on a server (Docker, Fly.io, or any host; see deploy/)
MARROW_TOKEN=$(openssl rand -hex 16) marrow-server

# on each device
export MARROW_REMOTE=https://your-backbone   MARROW_TOKEN=…   MARROW_PROJECT=team-app
```

Now a decision saved on your laptop is there on your desktop, instantly. Full walkthrough in
[deploy/README.md](deploy/README.md).

## More install options

Prebuilt binaries, no Rust:
```bash
curl -fsSL https://raw.githubusercontent.com/aryawidjaja/marrow/main/install.sh | sh
```
From source:
```bash
cargo install --git https://github.com/aryawidjaja/marrow marrow-cli marrow-mcp marrow-web
```
This puts `marrow`, `marrow-mcp`, and `marrow-serve` on your PATH.

## Bringing in an existing project

A fresh brain starts empty. To seed it from docs you already have, the first warm start nudges your
agent to run `marrow ingest`, it lists your README and `docs/` and distills them into memory. After
that, every session starts informed. Any time, run **`/marrow-save`** to capture the session you're in.

## Using Cursor, Codex, or other MCP agents

The automatic hooks are Claude Code specific, but any MCP agent gets the full memory toolset. Register
the server for every Claude Code project:
```bash
claude mcp add marrow -s user -- marrow-mcp --root .
```
For one project, add the same server to `.mcp.json` (Claude Code), `.cursor/mcp.json` (Cursor), or your
Codex TOML.

## Smarter (semantic) search

Search is keyword-based by default, instant and offline. For **meaning-based** recall (finding a note
about "JWT" when you search "login security"), install a semantic build:
```bash
brew install aryawidjaja/marrow/marrow-semantic   # multilingual, downloads a small model on first use
marrow embed fastembed
```
`marrow status` shows the mode; `marrow embed none` switches back. Semantic search also powers the
"related meaning" links in the dashboard graph.

## CLI

Your agent drives Marrow for you, but you can too:
```bash
marrow add --kind decision --topic auth "We use short-lived JWTs."   # save
marrow search "token expiry" --weight 1                              # find (0=keyword, 1=semantic)
marrow hub recall "rate limiting"                                    # search the whole hive
marrow list-stale --repo .                                           # notes whose code drifted
marrow consolidate --repo . --apply                                  # merge duplicates
marrow audit                                                         # prove the ledger untampered
```

`marrow add` writes a plain markdown file under `.marrow/memory/`, the YAML frontmatter is metadata,
the text below is the memory. The SQLite index is a rebuildable cache over these files.

## What's under the hood

- **Staleness detection**: a memory can cite a code symbol; Marrow fingerprints it and flags the note
  the moment the symbol changes, ignoring reformatting and renames.
- **Consolidation**: clusters related memories by meaning, merges duplicates, resolves contradictions,
  and retires expired notes, preserving lineage.
- **Hive mind**: many sessions work as one: each joins warm, claims its work so two never collide, and
  reads a live activity trail. Unlike a black-box hive, every signal is in an auditable ledger.
- **Audit & provenance**: every write, edit, and recall lands in an append-only, hash-chained ledger;
  any answer traces back to its sources.
- **Typed & validated**: five schemas (fact, decision, entity, session, skill); bad writes are rejected
  with reasons.
- **Runs anywhere**: offline single binaries; markdown is the source of truth, SQLite a disposable cache.

## The name

Marrow is where the immune system's memory begins, the quiet layer a body's knowledge is built on. In
Pluribus it is the marrow that keeps the immune themselves, the one thing the hive cannot take without
consent. Same idea here. Your agents share a memory, but it stays yours, in your marrow, on your terms.

## License

The engine (`crates/`) is **AGPL-3.0-only**; the embeddable Python backend (`python/marrow-anthropic`)
is **Apache-2.0**. Using Marrow from your agent over MCP or the CLI is a separate process, not a
derivative work. A commercial license is available, see [COMMERCIAL.md](COMMERCIAL.md).
