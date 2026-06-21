#!/usr/bin/env bash
# Stop hook: make semantic memory automatic, without nagging.
#
# When a session that actually changed code is about to end, this asks the agent — once — to
# persist any durable decisions/facts to Marrow, then lets it stop. Sessions that only read or
# chatted are NOT interrupted. This is what turns Marrow into a true background brain: the user
# never has to say "remember this".
#
# Fails open — any problem and it simply allows the session to stop.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "$root/target/release/marrow" ] && marrow="$root/target/release/marrow"

finish() {  # best-effort: record that the session ended, then allow stop
  [ -n "$marrow" ] && [ -d "$root/.marrow" ] \
    && "$marrow" --root "$root" log --kind finished --by claude-code "session ${1:-} ended" >/dev/null 2>&1 || true
  exit 0
}

input="$(cat 2>/dev/null || true)"

# Without jq we can't read the loop guard safely, so never block — just finish.
command -v jq >/dev/null 2>&1 || finish

active="$(printf '%s' "$input" | jq -r '.stop_hook_active // false')"
session="$(printf '%s' "$input" | jq -r '.session_id // "claude-code"')"
transcript="$(printf '%s' "$input" | jq -r '.transcript_path // empty')"

# Loop guard: if we already asked on this stop, let it finish now.
[ "$active" = "true" ] && finish "$session"

# Burden guard: only prompt if the session actually edited files. Read-only/Q&A sessions end clean.
edits=0
if [ -n "$transcript" ] && [ -f "$transcript" ]; then
  edits="$(grep -cE '"name":[[:space:]]*"(Edit|Write|MultiEdit|NotebookEdit)"' "$transcript" 2>/dev/null || true)"
  [ -z "$edits" ] && edits=0
fi
[ "$edits" -eq 0 ] 2>/dev/null && finish "$session"

# This session changed code — ask the agent to capture durable knowledge before stopping.
reason="Marrow memory check before you finish: this session changed code, so capture anything a future session should know. For each durable decision, fact, or gotcha from this session, call the mem_write tool (kind 'decision' or 'fact', a short topic, project 'default'). Be selective — skip routine edits and anything already in Marrow. If there is genuinely nothing new worth keeping, say so briefly and finish."
jq -n --arg r "$reason" '{decision:"block", reason:$r}'
exit 0
