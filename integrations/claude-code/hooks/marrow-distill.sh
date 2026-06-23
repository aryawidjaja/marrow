#!/usr/bin/env bash
# Stop hook (layer-2 capture): when a session has done enough new work, ask the agent, once, to save
# any durable decisions it hasn't saved yet before it goes idle. The in-session agent distills with
# full context, so this needs no separate model call and no transcript parsing. Throttled and
# loop-safe so it isn't noisy. Disable with MARROW_AUTODISTILL=0.
set -u
[ "${MARROW_AUTODISTILL:-1}" = "0" ] && exit 0
root="${CLAUDE_PROJECT_DIR:-.}"
command -v jq >/dev/null 2>&1 || exit 0
[ -d "$root/.marrow" ] || exit 0

input="$(cat 2>/dev/null || true)"
# Never re-trigger on the continuation we ourselves caused (prevents an infinite stop loop).
[ "$(printf '%s' "$input" | jq -r '.stop_hook_active // false')" = "true" ] && exit 0
session="$(printf '%s' "$input" | jq -r '.session_id // "session"')"
transcript="$(printf '%s' "$input" | jq -r '.transcript_path // empty')"
[ -n "$transcript" ] && [ -f "$transcript" ] || exit 0

# Throttle by transcript growth: only fire after a meaningful amount of new work since last capture.
every="${MARROW_DISTILL_EVERY:-40}"
wmdir="$root/.marrow/.distill"; mkdir -p "$wmdir" 2>/dev/null || exit 0
wm="$wmdir/$session"
last="$(cat "$wm" 2>/dev/null || echo 0)"
now="$(wc -l < "$transcript" 2>/dev/null | tr -d ' ')"
[ -n "$now" ] || exit 0
[ "$(( now - last ))" -lt "$every" ] && exit 0
echo "$now" > "$wm"

reason="Before you wrap up: save any durable decisions, facts, or gotchas from this session to Marrow that are not saved yet. For each, call mem_recall first to avoid duplicates, then mem_write (kind decision or fact, a short topic). Keep them concise and skip transient details. If everything worth keeping is already saved, just stop."
jq -n --arg r "$reason" '{decision:"block", reason:$r}'
exit 0
