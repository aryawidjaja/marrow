# Claude Code auto-capture hooks

These hooks make Marrow hands-free in Claude Code: every session starts warm, file edits stream
into the shared brain as progress, and sessions close themselves out — without spending model
tokens.

## Setup (one command)

From the project where you use Claude Code:
```bash
bash /path/to/marrow/integrations/claude-code/install.sh .
```
It creates the store, connects Marrow over MCP (`.mcp.json`), installs these hooks into
`.claude/hooks/`, and registers them in `.claude/settings.json`. Safe to re-run; it won't
overwrite an existing `settings.json`.

### Manual setup (if you prefer)

1. Make sure `marrow` and `marrow-mcp` are installed and `marrow init` has been run in your project.
2. Copy the hook scripts in:
   ```bash
   mkdir -p .claude/hooks
   cp integrations/claude-code/hooks/*.sh .claude/hooks/
   chmod +x .claude/hooks/*.sh
   ```
3. Merge [`settings.example.json`](settings.example.json) (the `hooks` block) into your
   `.claude/settings.json`, and add the MCP server to `.mcp.json` (see [../README.md](../README.md)).

The scripts use `jq` (preinstalled on most systems; `brew install jq` / `apt install jq`).
Every hook fails open — if anything goes wrong it exits 0 and never blocks your session.

## What each hook does

| Hook | Event | Effect |
|------|-------|--------|
| `marrow-bootstrap.sh` | `SessionStart` | Injects a warm-start briefing (active claims + relevant memories) so the session doesn't cold-start. Fires once per session. |
| `marrow-progress.sh` | `PostToolUse` (Edit/Write) | Records each file edit as a `progress` event other sessions can see live. |

This is what makes memory truly automatic and *unobtrusive*: the user never has to say "remember
this," and nothing pops up. The SessionStart hook reads the shared brain, `PostToolUse` records
file activity, and durable **decisions are saved in-flow** by the agent as it works (per the
project's `CLAUDE.md`). The agent can also call the MCP tools (`mem_write`, `mem_claim`, …) directly
any time.

(There is intentionally **no `Stop` hook**: Claude Code fires `Stop` after every agent turn, not at
session end, so anything it did would be noise or an interruption. Decision capture happens in-flow
instead.)
