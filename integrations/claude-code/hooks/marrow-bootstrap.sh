#!/usr/bin/env bash
# SessionStart hook: warm-start the session with a Marrow briefing (active claims + relevant
# memories) so it doesn't cold-start or re-scan. Fails open — never blocks the session.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

command -v marrow >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

brief="$(marrow --root "$root" bootstrap "resume work on this project" --by claude-code 2>/dev/null)" || exit 0
[ -n "$brief" ] || exit 0

if command -v jq >/dev/null 2>&1; then
  jq -n --arg c "$brief" \
    '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:("Marrow shared-brain briefing:\n"+$c)}}'
else
  printf 'Marrow shared-brain briefing:\n%s\n' "$brief"
fi
exit 0
