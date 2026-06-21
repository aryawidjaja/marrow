#!/usr/bin/env bash
# PreToolUse hook (Edit|Write|MultiEdit): make collision-avoidance automatic.
#
# Before the agent edits a file, this:
#   1. blocks the edit ONLY if the file is auto-claimed by a DIFFERENT session (a real
#      cross-session collision), then tells the agent to coordinate or pick different work;
#   2. otherwise auto-claims the file for THIS session.
#
# Auto-claims are tagged `[autoclaim]` and keyed to the real Claude Code session id, so the guard
# recognizes its own claims and never blocks an owner. Advisory *manual* claims (made via mem_claim
# with arbitrary labels) are intentionally IGNORED here — they can't be attributed to a session, so
# enforcing them would block the owner's own edits. Agents still see manual claims via the warm-start
# briefing / mem_claims and respect them by judgment.
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

# Claim lines look like: "<id>  [<session>]  <intent>". Only the guard's own auto-claims
# (intent tagged "[autoclaim]") from a DIFFERENT session count as a hard collision.
others="$(printf '%s' "$claims" | grep -F '[autoclaim]' | grep -vF "[$session]" || true)"
if [ -n "$others" ]; then
  printf 'Marrow: another active session is editing %s — do not edit it in parallel. Coordinate, pick different work, or wait for its lease to expire.\n' "$rel" >&2
  exit 2   # blocks this edit and shows the reason to the agent
fi

# No collision — auto-claim the file for this session (once) so other sessions see it.
if ! printf '%s' "$claims" | grep -F '[autoclaim]' | grep -qF "[$session]"; then
  "$marrow" --root "$root" claim "[autoclaim] $rel" --session "$session" --file "$rel" --by marrow-guard >/dev/null 2>&1 || true
fi
exit 0
