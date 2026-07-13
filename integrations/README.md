# Integrating Marrow with your agents

Marrow is meant to be as easy to adopt as a database: install it, point your agent at it, done.
There are two layers of integration — start with the first, add the second when you want it
hands-free.

1. **MCP (everyone starts here).** Marrow ships an MCP server, so any MCP-capable agent —
   Claude Code, Cursor, Codex, or your own — gets all of Marrow's tools by adding one block of
   config. No code.
2. **Auto-capture hooks (optional).** Wire Marrow into your agent's lifecycle so memory is
   recorded and recalled automatically, with no tokens spent asking the model to do it.

---

## 1. Install

```bash
cargo install --path crates/marrow-cli      # the `marrow` CLI
cargo install --path crates/marrow-mcp      # the `marrow-mcp` server
marrow init                                 # create .marrow/ in your project
```

(Or use the release binaries from `cargo build --release` at `target/release/`.)

---

## 2. Connect your agent (MCP)

The same server works everywhere. Point it at your project root with `--root`.

### Claude Code
Add to `.mcp.json` in your project (or `~/.claude.json`):
```json
{
  "mcpServers": {
    "marrow": { "command": "marrow-mcp", "args": ["--root", "."] }
  }
}
```

### Cursor
Add to `.cursor/mcp.json`:
```json
{
  "mcpServers": {
    "marrow": { "command": "marrow-mcp", "args": ["--root", "."] }
  }
}
```

### Codex / OpenAI-style agents (TOML config)
```toml
[mcp_servers.marrow]
command = "marrow-mcp"
args = ["--root", "."]
```

### Any other agent
`marrow-mcp` speaks MCP over stdio (spec 2025-06-18). Launch `marrow-mcp --root /path/to/project`
and connect it like any stdio MCP server. Self-hosted, no network, nothing leaves the machine.

---

## 3. Tell the agent how to use it (recommended workflow)

Drop this into your agent's system prompt / `CLAUDE.md` so it uses the shared brain well:

```
You share a Marrow memory with other agent sessions. Follow this loop:
1. At the start of a task, call mem_bootstrap with your goal. Read the briefing: it tells you
   what other sessions are already doing and the memories/decisions relevant to your goal.
   Do NOT re-scan the whole codebase first.
2. Before you answer anything about how this project works, call mem_recall. It returns the
   matches AND the memories connected to them. A neighbour with `hops` of 2 or more matched none
   of your words — the brain followed the links to reach it, so read it.
3. When you learn something durable (a decision, a fact, a gotcha), write it with mem_write so
   the next session inherits it. Call mem_areas first and file it into an area that already
   exists, keep `topic` a short label, and reference related memories as [[links]] in the body —
   links are how an old memory stays findable once nobody searches for its words any more.
4. When a decision changes, mem_supersede the old one instead of writing a contradiction.
```

Claiming files so two sessions never collide is handled by the hooks, not by the agent: it happens
for you. That loop is the whole point: **one brain, many hands.**

---

## 4. Auto-capture (optional, zero-token)

So you don't rely on the model remembering the loop, wire Marrow into the agent lifecycle. For
Claude Code, copy the hooks in [`claude-code/`](claude-code/) — they bootstrap context on session
start, stream file edits as progress, and close the session out, all without spending model
tokens. See [`claude-code/README.md`](claude-code/README.md).

---

## The tools you get (MCP)

Your agent is given a small set, so it spends its tokens on your code and not on reading a tool
catalogue: `mem_bootstrap` (start warm) · `mem_recall` (the matches *and* what they're connected to) ·
`mem_search` · `mem_areas` · `mem_write` · `mem_read` · `mem_supersede` · `mem_ingest`.

Join a hive and it also gets `mem_hub_recall`, `mem_hub_activity`, and the agent channel
(`mem_ask` · `mem_inbox` · `mem_reply`).

Claiming files so two sessions don't collide happens in the hooks, not in the agent: it is handled
for you rather than being something the agent has to remember to do.

Everything has a `marrow <command>` CLI equivalent, so scripts and hooks can do the same.
