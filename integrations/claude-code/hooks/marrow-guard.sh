#!/usr/bin/env bash
# PreToolUse hook (Edit|Write|MultiEdit): make collision-avoidance automatic.
#
# Before the agent edits a file, this:
#   1. checks Marrow for an active work-claim on that file held by ANOTHER session — if found,
#      it blocks the edit and tells the agent to coordinate or pick different work;
#   2. otherwise auto-claims the file for THIS session so other sessions see what it's doing.
#
# The user never has to say "claim this" and the agent never has to remember to — coordination
# just happens. Fails OPEN: any problem and the edit is allowed (never blocks real work by mistake).
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "$root/target/release/marrow" ] && marrow="$root/target/release/marrow"
[ -z "$marrow" ] && exit 0
command -v jq >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

input="$(cat 2>/dev/null || true)"
file="$(printf '%s' "$input" | jq -r '.tool_input.file_path // .tool_input.path // empty')"
session="$(printf '%s' "$input" | jq -r '.session_id // "claude-code"')"
[ -z "$file" ] && exit 0

# Normalize to a repo-relative path so claims match regardless of absolute/relative form.
case "$file" in "$root"/*) rel="${file#"$root"/}";; *) rel="$file";; esac

claims="$("$marrow" --root "$root" claims --file "$rel" 2>/dev/null || true)"

# Claim lines look like: "<id>  [<session>]  <intent>". A conflict is any such line from a
# DIFFERENT session.
others="$(printf '%s' "$claims" | grep '\[' | grep -vF "[$session]" || true)"
if [ -n "$others" ]; then
  intent="$(printf '%s' "$others" | head -1 | sed -E 's/^[^[]*\[[^]]*\][[:space:]]*//')"
  printf 'Marrow: another agent session has an active claim on %s (intent: %s). Do NOT edit it in parallel — coordinate, pick different work, or wait for the claim to expire/release.\n' "$rel" "$intent" >&2
  exit 2   # blocks this edit and shows the reason to the agent
fi

# No conflict — auto-claim the file for this session (once) so others can see it.
if ! printf '%s' "$claims" | grep -qF "[$session]"; then
  "$marrow" --root "$root" claim "editing $rel" --session "$session" --file "$rel" >/dev/null 2>&1 || true
fi
exit 0
