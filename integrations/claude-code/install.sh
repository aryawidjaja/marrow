#!/usr/bin/env bash
# One-command Marrow setup for a project using Claude Code.
#
# It connects Marrow over MCP and installs the auto-capture hooks so every session
# bootstraps warm, streams progress, and closes out — with no prompting and no model tokens.
#
# Usage:
#   bash install.sh [TARGET_PROJECT_DIR]     # defaults to the current directory
#
# Safe to re-run: it never overwrites an existing .claude/settings.json (it writes a
# .marrow.json beside it for you to merge instead).
set -euo pipefail

target="${1:-$PWD}"
here="$(cd "$(dirname "$0")" && pwd)"   # integrations/claude-code

if ! command -v marrow >/dev/null 2>&1 || ! command -v marrow-mcp >/dev/null 2>&1; then
  echo "✗ Marrow isn't installed. From the Marrow repo, run:"
  echo "    cargo install --path crates/marrow-cli --force"
  echo "    cargo install --path crates/marrow-mcp --force"
  exit 1
fi

cd "$target"
echo "Setting up Marrow in: $target"

# 1) The store
[ -d .marrow ] || { marrow init >/dev/null && echo "  ✓ created .marrow store"; }

# 2) Connect Marrow over MCP. Prefer registering it once at USER scope so it's available in every
# project. Use the ABSOLUTE path: the spawn shell may not have ~/.cargo/bin on PATH.
mcp_bin="$(command -v marrow-mcp)"
if command -v claude >/dev/null 2>&1; then
  if claude mcp add marrow -s user -- "$mcp_bin" --root . >/dev/null 2>&1; then
    echo "  ✓ registered marrow MCP at user scope (available in every project)"
  else
    echo "  • marrow MCP already registered at user scope — left as-is"
  fi
elif [ ! -f .mcp.json ]; then
  printf '{ "mcpServers": { "marrow": { "command": "%s", "args": ["--root", "."] } } }\n' "$mcp_bin" > .mcp.json
  echo "  ✓ wrote .mcp.json (project-scoped MCP connection -> $mcp_bin)"
else
  echo "  • .mcp.json already exists — left as-is"
fi

# 3) Auto-capture hooks
mkdir -p .claude/hooks
cp "$here"/hooks/*.sh .claude/hooks/
chmod +x .claude/hooks/*.sh
echo "  ✓ installed hooks into .claude/hooks/"

# 4) Register the hooks
if [ ! -f .claude/settings.json ]; then
  cp "$here/settings.example.json" .claude/settings.json
  echo "  ✓ wrote .claude/settings.json (hook registration)"
else
  cp "$here/settings.example.json" .claude/settings.marrow.json
  echo "  • .claude/settings.json exists — wrote .claude/settings.marrow.json; merge its \"hooks\" block in"
fi

# 5) Drop a tiny, idempotent agent-guidance block into CLAUDE.md so the user authors NOTHING.
# The hooks already handle warm-start, collision-avoidance, and activity automatically; this only
# nudges the agent to save durable *decisions* (a judgment call a hook can't make).
guidance() {
  cat <<'MD'
<!-- marrow:begin (managed by Marrow — installed by integrations/claude-code/install.sh) -->
## Marrow shared memory

This project has a Marrow shared brain connected over MCP. Hooks already bootstrap context at
session start, prevent file collisions before edits, and record your activity — automatically.
You only need to do one thing: **when you reach a durable decision, fact, or gotcha, save it with
the `mem_write` tool** (kind `decision`/`fact`, a short topic, project `default`), so the next
session inherits it. Use `mem_recall` before answering questions about past decisions, and don't
re-save anything already in Marrow.
<!-- marrow:end -->
MD
}
if [ ! -f CLAUDE.md ]; then
  guidance > CLAUDE.md
  echo "  ✓ wrote CLAUDE.md (agent guidance)"
elif grep -q "marrow:begin" CLAUDE.md; then
  echo "  • CLAUDE.md already has the Marrow block — left as-is"
else
  { printf '\n'; guidance; } >> CLAUDE.md
  echo "  ✓ appended the Marrow block to your existing CLAUDE.md"
fi

cat <<'EOF'

Done. Open Claude Code in this project — each session will now:
  • bootstrap from Marrow at start (warm, no re-scan),
  • record file edits as progress,
  • and close the session out.
Tip: add .marrow/ .mcp.json .claude/ to your .gitignore if they're local-only.
EOF
