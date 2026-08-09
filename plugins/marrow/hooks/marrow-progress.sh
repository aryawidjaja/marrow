#!/usr/bin/env bash
# PostToolUse hook (Edit/Write/MultiEdit): record the edited file as a progress event so other
# sessions see it in the shared activity stream. Fails open — never blocks the tool.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "$root/target/release/marrow" ] && marrow="$root/target/release/marrow"
[ -z "$marrow" ] && exit 0
command -v jq >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

input="$(cat)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty')"
session="$(printf '%s' "$input" | jq -r '.session_id // "claude-code"')"
[ -n "$file" ] || exit 0

"$marrow" --root "$root" progress "edited $file" --session "$session" --file "$file" --by claude-code \
  >/dev/null 2>&1 || true
exit 0
