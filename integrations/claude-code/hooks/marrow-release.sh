#!/usr/bin/env bash
# Stop hook: when this session goes idle (finished a turn), release the file claims it auto-grabbed
# while editing, so other sessions aren't blocked on files it's no longer touching. Active work
# re-claims on the next edit; a claim's TTL is the backstop if a session dies. Always on, fail-open.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"
marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "/opt/homebrew/bin/marrow" ] && marrow="/opt/homebrew/bin/marrow"
[ -z "$marrow" ] && exit 0
command -v jq >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

input="$(cat 2>/dev/null || true)"
# Skip the continuation another Stop hook may have triggered; we already released on the real stop.
[ "$(printf '%s' "$input" | jq -r '.stop_hook_active // false')" = "true" ] && exit 0
session="$(printf '%s' "$input" | jq -r '.session_id // empty')"
[ -n "$session" ] || exit 0

"$marrow" --root "$root" release --session "$session" --by marrow-guard >/dev/null 2>&1 || true
exit 0
