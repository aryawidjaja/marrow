#!/usr/bin/env bash
# SessionStart hook: warm-start the session with a Marrow briefing (active claims + relevant
# memories) so it doesn't cold-start or re-scan. Fails open — never blocks the session.
set -u
root="${CLAUDE_PROJECT_DIR:-.}"

# Locate the marrow binary. Hooks run in a non-login shell that may NOT have ~/.cargo/bin on
# PATH, so check the usual install locations explicitly.
marrow="$(command -v marrow || true)"
[ -z "$marrow" ] && [ -x "$HOME/.cargo/bin/marrow" ] && marrow="$HOME/.cargo/bin/marrow"
[ -z "$marrow" ] && [ -x "$root/target/release/marrow" ] && marrow="$root/target/release/marrow"
[ -z "$marrow" ] && exit 0
[ -d "$root/.marrow" ] || exit 0

# Hands-free maintenance: run consolidation only if enough new memories have piled up since the
# last pass (a no-op otherwise). Keeps the brain coherent without the user lifting a finger.
"$marrow" --root "$root" consolidate --repo "$root" --if-due >/dev/null 2>&1 || true

brief="$("$marrow" --root "$root" bootstrap "resume work on this project" --by claude-code 2>/dev/null)" || exit 0
[ -n "$brief" ] || exit 0

if command -v jq >/dev/null 2>&1; then
  jq -n --arg c "$brief" \
    '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:("Marrow shared-brain briefing:\n"+$c)}}'
else
  printf 'Marrow shared-brain briefing:\n%s\n' "$brief"
fi
exit 0
