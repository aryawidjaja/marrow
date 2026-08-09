# Marrow — Claude Code plugin

Shared memory and collision-avoidance for parallel Claude Code sessions.

```
/plugin marketplace add aryawidjaja/marrow
/plugin install marrow@marrow
```

## Prerequisites

The plugin wires Claude Code up to the `marrow` binaries; it does not contain them.

```bash
brew install aryawidjaja/marrow/marrow      # macOS / Linux
irm marrow.works/install.ps1 | iex          # Windows
```

The hooks are shell scripts and need `jq`. Without `jq` the memory tools still work and the
automatic coordination stays off.

## What it adds

| Component | Effect |
|---|---|
| MCP server | The `mem_*` tools: recall, write, supersede, areas, rooms |
| `SessionStart` hook | Warm-starts the session with relevant memory and what other sessions are doing |
| `PreToolUse` hook | Blocks an edit to a file another live session has claimed |
| `PostToolUse` hook | Records the edit so other sessions see it, and renews this session's claim |
| `Stop` hook | Releases claims and saves anything durable from the session |
| `/marrow:marrow-save` | Save this session's decisions on demand |

## Turning it off

Disable in `/plugin`, or remove it entirely:

```
/plugin uninstall marrow@marrow
```

Your memories live in `.marrow/` in the project and are not touched by uninstalling.
