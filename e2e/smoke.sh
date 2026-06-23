#!/usr/bin/env bash
#
# End-to-end smoke test: builds the real binaries and exercises the whole product
# (CLI + MCP server) against a sample project, asserting on every step.
#
# Usage: e2e/smoke.sh

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

pass=0
check() { # check "<description>" "<haystack>" "<needle>"
  if printf '%s' "$2" | grep -qF -- "$3"; then
    printf '  ok   %s\n' "$1"
    pass=$((pass + 1))
  else
    printf '  FAIL %s\n    expected to find: %s\n    in: %s\n' "$1" "$3" "$2"
    exit 1
  fi
}

absent() { # absent "<description>" "<haystack>" "<needle>"
  if printf '%s' "$2" | grep -qF -- "$3"; then
    printf '  FAIL %s\n    did not expect: %s\n    in: %s\n' "$1" "$3" "$2"
    exit 1
  fi
  printf '  ok   %s\n' "$1"
  pass=$((pass + 1))
}

echo "==> Building release binaries"
cargo build --release --quiet --manifest-path "$repo_root/Cargo.toml"
marrow="$repo_root/target/release/marrow"
marrow_mcp="$repo_root/target/release/marrow-mcp"

echo "==> Creating a sample project"
proj="$work/project"
mkdir -p "$proj/src"
cat > "$proj/src/auth.rs" <<'RS'
pub fn issue_token(user: &str) -> String {
    format!("jwt:{user}")
}
RS
cat > "$proj/src/ratelimit.rs" <<'RS'
pub fn allow(requests: u32) -> bool {
    requests <= 100
}
RS

m() { "$marrow" --root "$proj" "$@"; }

echo "==> CLI: init and write memories"
check "init creates the store" "$(m init)" "Initialized"
check "add a plain decision" "$(m add --kind decision --topic storage 'Memories are stored as markdown files.')" ""
auth_id="$(m anchor --kind decision --topic auth --repo "$proj" --file src/auth.rs --symbol issue_token 'Auth issues a signed JWT string.' | tr -d '[:space:]')"
check "anchored auth decision got an id" "$auth_id" "01"
m anchor --kind fact --topic ratelimit --repo "$proj" --file src/ratelimit.rs --symbol allow 'The limiter allows up to 100 requests.' > /dev/null

echo "==> CLI: query and search"
check "query returns active decisions" "$(m query --kind decision)" "Auth issues a signed JWT"
check "full-text search finds a memory" "$(m search markdown)" "stored as markdown"
check "search miss returns nothing" "$(m search kubernetes)" "0 result(s)"
check "status counts memories" "$(m status)" "total: 3"

echo "==> CLI: validation rejects a bad write"
bad="$(m add --kind decision 'a decision with no topic' 2>&1 || true)"
check "decision without topic is rejected" "$bad" "topic"

echo "==> CLI: supersede keeps one active decision per topic"
new_id="$(m supersede "$auth_id" --kind decision --topic auth 'Auth now issues opaque tokens.' | tr -d '[:space:]')"
check "superseding query shows only the new decision" "$(m query --kind decision --topic auth)" "$new_id"
check "old decision is no longer active" "$(m query --kind decision --topic auth)" "1 result(s)"

echo "==> Staleness: the headline feature"
check "fresh anchors are not stale" "$(m list-stale --repo "$proj")" "0 stale anchor(s)"
# Change what issue_token actually does.
cat > "$proj/src/auth.rs" <<'RS'
pub fn issue_token(user: &str) -> String {
    format!("opaque:{user}:v2")
}
RS
stale="$(m list-stale --repo "$proj")"
check "changed code flags exactly one stale anchor" "$stale" "1 stale anchor(s)"
check "the stale anchor names the changed symbol" "$stale" "issue_token"
absent "the unrelated rate-limit anchor stays valid" "$stale" "allow"

echo "==> CLI: doctor rebuilds the index from files"
rm -f "$proj/.marrow/.index/sqlite.db"
check "doctor reindexes every memory" "$(m doctor)" "Reindexed"
check "memories survive a reindex" "$(m status)" "total:"

echo "==> MCP server: a real stdio session"
session="$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mem_search","arguments":{"text":"markdown"}}}' \
  '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mem_list_stale","arguments":{}}}' \
  | "$marrow_mcp" --root "$proj")"
check "MCP handshake reports the protocol version" "$session" '"protocolVersion":"2025-06-18"'
check "MCP advertises the anchor tool" "$session" "mem_anchor"
check "MCP search returns the stored memory" "$session" "stored as markdown"
check "MCP reports the stale anchor" "$session" "issue_token"

echo "==> Audit ledger: tamper-evident history"
m log --kind observe "noticed the auth change in review" > /dev/null
check "history records writes and observations" "$(m history)" "observe"
check "audit chain verifies clean" "$(m audit)" "audit ok"
# Tamper with a recorded summary on disk; the hash chain must catch it.
log_file="$proj/.marrow/episodic/log.jsonl"
sed -i.bak 's/noticed/forged/' "$log_file" 2>/dev/null || sed -i 's/noticed/forged/' "$log_file"
tampered_audit="$("$marrow" --root "$proj" audit 2>&1 || true)"
check "tampering breaks the audit chain" "$tampered_audit" "broken"

echo "==> Consolidation: detect and distill"
proj2="$work/project2"
mkdir -p "$proj2"
n() { "$marrow" --root "$proj2" "$@"; }
n init > /dev/null
n add --kind fact --topic a "the cache is invalidated on write" > /dev/null
n add --kind fact --topic b "the cache is invalidated on write" > /dev/null
check "consolidation detects the duplicate" "$(n consolidate --repo "$proj2")" "related memories: 1"
check "applying consolidation merges it" "$(n consolidate --repo "$proj2" --apply)" "1 merged"
check "no duplicates remain after merge" "$(n consolidate --repo "$proj2")" "related memories: 0"

echo "==> Coordination plane: one brain, many hands"
proj3="$work/project3"
mkdir -p "$proj3"
c() { "$marrow" --root "$proj3" "$@"; }
c init > /dev/null
claim_id="$(c claim "refactor auth" --session a --file src/auth.rs --project demo | tr -d '[:space:]')"
check "a claim is registered" "$(c claims)" "refactor auth"
check "a second agent sees the overlapping claim" "$(c claims --file src/auth.rs --project demo)" "1 active claim(s)"
check "a non-overlapping scope is free" "$(c claims --file src/billing.rs --project demo)" "0 active claim(s)"
c progress "wrote token issuer" --session a --file src/auth.rs > /dev/null
check "progress shows in the activity stream" "$(c activity)" "wrote token issuer"
c release "$claim_id" > /dev/null
check "released claims are no longer active" "$(c claims)" "0 active claim(s)"
# Releasing by session frees everything that session holds (auto-release on idle).
c claim "edit one" --session sx --file src/one.rs --project demo > /dev/null
c claim "edit two" --session sx --file src/two.rs --project demo > /dev/null
check "release --session frees all its claims" "$(c release --session sx)" "released 2 claim(s)"
check "no claims linger after session release" "$(c claims)" "0 active claim(s)"
check "bootstrap warm-starts a session" "$(c bootstrap 'work on auth' --project demo)" "goal: work on auth"
coord_session="$(printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mem_claim","arguments":{"session":"x","intent":"db work","files":["src/db.rs"],"project":"demo"}}}' \
  | "$marrow_mcp" --root "$proj3")"
check "MCP exposes the coordination tools" "$coord_session" '"id"'

echo "==> Hive-mind: cross-session awareness (watch)"
proj_w="$work/projectw"
mkdir -p "$proj_w"
w() { "$marrow" --root "$proj_w" "$@"; }
w init > /dev/null
w claim "refactor auth" --session other --file src/auth.rs --project demo > /dev/null
w progress "edited the parser" --session other --file src/parser.rs > /dev/null
watch1="$(w watch --session me)"
check "watch surfaces another session's claim" "$watch1" "refactor auth"
check "watch surfaces another session's edit" "$watch1" "edited the parser"
check "watch notes active foreign claims" "$(w watch --session me)" "other session(s) hold active claims"

echo "==> Onboarding: seed an existing repo + capture prompt"
proj4="$work/project4"
mkdir -p "$proj4/docs"
echo "# Read me" > "$proj4/README.md"
echo "architecture notes" > "$proj4/docs/architecture.md"
o() { "$marrow" --root "$proj4" "$@"; }
o init > /dev/null
ingest_out="$(o ingest)"
check "ingest lists the existing README" "$ingest_out" "README.md"
check "ingest lists docs/ markdown" "$ingest_out" "docs/architecture.md"
check "ingest tells the agent to distill via mem_write" "$ingest_out" "mem_write"
check "empty-brain bootstrap nudges ingest" "$(o bootstrap 'resume' --project demo)" "marrow ingest"
onboard_session="$(printf '%s\n%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"prompts/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mem_ingest","arguments":{}}}' \
  | "$marrow_mcp" --root "$proj4")"
check "MCP advertises the save prompt" "$onboard_session" '"save"'
check "MCP mem_ingest returns the docs" "$onboard_session" "README.md"

echo "==> Setup: scaffolds hooks, slash command, guidance"
proj5="$work/project5"
mkdir -p "$proj5/.claude"
# Pre-existing settings.json with the user's own config — setup must MERGE, not clobber.
printf '{"model":"opus"}' > "$proj5/.claude/settings.json"
setup_out="$("$marrow" --root "$proj5" setup 2>&1 || true)"
check "setup reports the slash command" "$setup_out" "marrow-save.md"
check "setup wrote the /marrow-save command" "$(cat "$proj5/.claude/commands/marrow-save.md")" "mem_write"
check "setup installs the auto-distill hook" "$(ls "$proj5/.claude/hooks/")" "marrow-distill.sh"
check "settings wires the Stop hook" "$(cat "$proj5/.claude/settings.json")" "marrow-distill.sh"
check "guidance block tells the agent to recall + save" "$(cat "$proj5/CLAUDE.md")" "Save as you go"
merged="$(cat "$proj5/.claude/settings.json")"
check "setup merged hooks into existing settings.json" "$merged" "marrow-bootstrap.sh"
check "setup preserved the user's existing settings" "$merged" '"model"'

echo "==> Bare invocation: getting-started banner"
check "marrow with no args points to setup" "$("$marrow")" "marrow setup"

echo "==> Auto-consolidation: threshold-driven cleanup"
proj6="$work/project6"
mkdir -p "$proj6"
a() { "$marrow" --root "$proj6" "$@"; }
a init > /dev/null
check "below threshold, --if-due is a no-op" "$(a consolidate --if-due)" "not due"
for i in $(seq 1 20); do a add --kind fact --topic "t$i" "memory body number $i" > /dev/null; done
check "past threshold, bootstrap nudges consolidation" "$(a bootstrap 'resume' --project demo)" "maintenance:"
check "past threshold, --if-due applies" "$(a consolidate --if-due)" "applied:"
check "after a pass, --if-due is a no-op again" "$(a consolidate --if-due)" "not due"

printf '\nAll %d checks passed.\n' "$pass"
