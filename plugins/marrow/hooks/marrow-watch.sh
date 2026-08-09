#!/usr/bin/env bash
# UserPromptSubmit hook: keep this session aware of what OTHER sessions are doing, every turn.
# Injects a short delta (new claims/edits from others) only when there's cross-session activity.
# Fails open.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && exit 0
command -v jq >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

input="$(cat 2>/dev/null || true)"
session="$(printf '%s' "$input" | jq -r '.session_id // "claude-code"')"

delta="$("$marrow" --root "$root" watch --session "$session" 2>/dev/null || true)"
[ -n "$delta" ] || exit 0

jq -n --arg c "$delta" '{hookSpecificOutput:{hookEventName:"UserPromptSubmit",additionalContext:$c}}'
exit 0
