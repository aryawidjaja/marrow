#!/usr/bin/env bash
# Stop hook (non-blocking, invisible): record that the session ended so the activity stream is
# complete. It never blocks the agent and never shows the user anything — memory capture must be
# background, not an interruption.
#
# Durable *decisions* are captured in-flow during the session (the agent calls mem_write as it
# works; see CLAUDE.md), not by nagging at the end. Mechanical activity (file edits) is already
# captured by the PostToolUse hook. So this hook just closes the session out, quietly.
#
# Fails open — any problem and it simply allows the session to stop.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "$root/target/release/marrow" ] && marrow="$root/target/release/marrow"
[ -z "$marrow" ] && exit 0
[ -d "$root/.marrow" ] || exit 0

session="claude-code"
if command -v jq >/dev/null 2>&1; then
  session="$(cat | jq -r '.session_id // "claude-code"' 2>/dev/null || echo claude-code)"
fi

"$marrow" --root "$root" log --kind finished --by claude-code "session $session ended" >/dev/null 2>&1 || true
exit 0
