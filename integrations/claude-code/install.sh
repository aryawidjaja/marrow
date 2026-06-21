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

# 2) Connect Marrow over MCP. Use the ABSOLUTE path to marrow-mcp: Claude Code spawns MCP
# servers in a shell that may not have ~/.cargo/bin on PATH, so a bare "marrow-mcp" can fail
# to connect.
mcp_bin="$(command -v marrow-mcp)"
if [ ! -f .mcp.json ]; then
  printf '{ "mcpServers": { "marrow": { "command": "%s", "args": ["--root", "."] } } }\n' "$mcp_bin" > .mcp.json
  echo "  ✓ wrote .mcp.json (MCP connection -> $mcp_bin)"
else
  echo "  • .mcp.json already exists — left as-is (ensure it points to: $mcp_bin)"
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

cat <<'EOF'

Done. Open Claude Code in this project — each session will now:
  • bootstrap from Marrow at start (warm, no re-scan),
  • record file edits as progress,
  • and close the session out.
Tip: add .marrow/ .mcp.json .claude/ to your .gitignore if they're local-only.
EOF
