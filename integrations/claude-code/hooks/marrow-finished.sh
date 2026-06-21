#!/usr/bin/env bash
# Stop hook: mark the session finished in the activity stream. Fails open.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

command -v marrow >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

session="claude-code"
if command -v jq >/dev/null 2>&1; then
  session="$(cat | jq -r '.session_id // "claude-code"' 2>/dev/null || echo claude-code)"
fi

marrow --root "$root" log --kind finished --by claude-code "session $session ended" \
  >/dev/null 2>&1 || true
exit 0
