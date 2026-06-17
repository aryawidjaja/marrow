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

printf '\nAll %d checks passed.\n' "$pass"
