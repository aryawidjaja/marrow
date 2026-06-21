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
2. Before you start editing, call mem_claims with the files/feature you intend to change.
   If an active claim overlaps, pick different work or coordinate — don't collide.
3. Claim your work with mem_claim, then record notable steps with mem_progress.
4. When you learn something durable (a decision, a fact, a gotcha), write it with mem_write so
   the next session inherits it.
5. Release your claim with mem_release when done.
```

That five-step loop is the whole point: **one brain, many hands.**

---

## 4. Auto-capture (optional, zero-token)

So you don't rely on the model remembering the loop, wire Marrow into the agent lifecycle. For
Claude Code, copy the hooks in [`claude-code/`](claude-code/) — they bootstrap context on session
start, stream file edits as progress, and close the session out, all without spending model
tokens. See [`claude-code/README.md`](claude-code/README.md).

---

## The tools you get (MCP)

Knowledge plane: `mem_write` · `mem_anchor` · `mem_read` · `mem_query` · `mem_search` ·
`mem_recall` · `mem_provenance` · `mem_supersede` · `mem_list_stale` · `mem_validate` ·
`mem_status` · `mem_history` · `mem_audit` · `mem_consolidate` · `mem_log`

Coordination plane (the shared brain): `mem_bootstrap` · `mem_claim` · `mem_claims` ·
`mem_release` · `mem_progress` · `mem_activity`

Everything has a `marrow <command>` CLI equivalent, so scripts and hooks can do the same.
