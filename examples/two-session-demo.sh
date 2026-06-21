#!/usr/bin/env bash
# Marrow shared-brain demo: two agent sessions, one brain.
#
# Shows the thing nobody else has — a coordination plane across independent sessions:
# Session B sees Session A's work-claim and picks different work; a later session
# bootstraps WARM from what A learned, with no re-scan. Everything is in one audit ledger.
#
# Run from the repo root:  bash examples/two-session-demo.sh
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
marrow="$here/target/release/marrow"
[ -x "$marrow" ] || { echo "building release binary..."; (cd "$here" && cargo build --release -q -p marrow-cli); }

work="$(mktemp -d)"; trap 'rm -rf "$work"' EXIT
proj="$work/app"; mkdir -p "$proj"
m() { "$marrow" --root "$proj" "$@"; }

say() { printf '\n\033[1;36m%s\033[0m\n' "$*"; }    # cyan headers
run() { printf '\033[2m$ marrow %s\033[0m\n' "$*"; m "$@"; }

say "0. One shared brain for the project"
m init >/dev/null
echo "store created at \$PROJECT/.marrow (markdown files + index + audit ledger)"

say "1. Session A starts — bootstraps, then claims the auth refactor"
run bootstrap "refactor authentication" --project app --by session-A
run claim "refactor auth to async" --session session-A --file "src/auth.rs" --project app

say "2. Session B starts in PARALLEL — checks before touching auth"
echo "B intends to also work on src/auth.rs, so it checks first:"
run claims --file "src/auth.rs" --project app
echo "--> B sees A already owns it. B picks DIFFERENT work instead of colliding:"
run claim "add billing module" --session session-B --file "src/billing.rs" --project app

say "3. A makes progress and records what it learned (so others inherit it)"
run progress "extracted token issuer into issue_token()" --session session-A --file "src/auth.rs"
run add --kind decision --topic auth --project app --by session-A "Auth uses short-lived async JWTs; refresh via /auth/refresh."

say "4. A finishes and releases its claim"
a_claim="$(m claims --file src/auth.rs --project app | awk 'NR==1{print $1}')"
run release "$a_claim"

say "5. A fresh session C arrives later — and starts WARM, not cold"
echo "C doesn't re-scan the repo; it just asks the shared brain:"
run bootstrap "continue the auth work" --project app --by session-C
echo "--> C already knows A's decision and that auth is now free to work on."

say "6. The whole thing is one tamper-evident activity stream"
run activity --limit 8
run audit

say "Done. Two sessions, one brain: no collision, no re-learning, no cold start."
