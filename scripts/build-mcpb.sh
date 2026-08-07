#!/usr/bin/env bash
# Build the MCPB bundle for marrow-mcp and the server.json that points at it.
#
# An MCPB ("MCP Bundle") is a single zip a client can install with no toolchain
# — which is the whole point for us, since the alternative is asking people to
# install Rust. One bundle carries every platform's binary; the manifest picks
# the right one at launch.
#
#   ./scripts/build-mcpb.sh 0.7.0 [dir-with-binaries]
#
# The registry caps server.json's description at 100 characters — keep it short
# here or `mcp-publisher publish` fails with a 422.
#
# Binaries are looked for as <dir>/<target>/marrow-mcp. With no dir, it builds
# for the host only, which is enough to validate packaging locally but is NOT
# what ships — releases must carry all three.
set -euo pipefail

VERSION="${1:?usage: build-mcpb.sh <version> [binary-dir]}"
VERSION="${VERSION#v}"
BIN_DIR="${2:-}"
REPO="https://github.com/aryawidjaja/marrow"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/dist/mcpb"
STAGE="$OUT/bundle"
BUNDLE="$OUT/marrow-mcp.mcpb"

rm -rf "$OUT"
mkdir -p "$STAGE/server"

say() { printf '\033[1m›\033[0m %s\n' "$*"; }

# ── collect the binaries ────────────────────────────────────────────────────
# macOS ships as one universal binary rather than two files: the manifest can
# switch on platform (darwin/win32/linux) but not on architecture, so arm64 and
# x86_64 have to be fused with lipo.
if [[ -n "$BIN_DIR" ]]; then
  arm="$BIN_DIR/aarch64-apple-darwin/marrow-mcp"
  intel="$BIN_DIR/x86_64-apple-darwin/marrow-mcp"
  linux="$BIN_DIR/x86_64-unknown-linux-gnu/marrow-mcp"

  for f in "$arm" "$intel" "$linux"; do
    [[ -f "$f" ]] || { echo "missing binary: $f" >&2; exit 1; }
  done

  say "fusing macOS arm64 + x86_64 into a universal binary"
  lipo -create "$arm" "$intel" -output "$STAGE/server/marrow-mcp-darwin"
  cp "$linux" "$STAGE/server/marrow-mcp-linux"
  PLATFORMS='["darwin", "linux"]'
else
  say "no binary dir given — building for the host only (local validation)"
  cargo build --release --bin marrow-mcp --manifest-path "$ROOT/Cargo.toml"
  case "$(uname -s)" in
    Darwin) host=darwin ;;
    Linux)  host=linux ;;
    *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
  esac
  cp "$ROOT/target/release/marrow-mcp" "$STAGE/server/marrow-mcp-$host"
  PLATFORMS="[\"$host\"]"
fi
chmod +x "$STAGE/server/"*

# ── manifest ────────────────────────────────────────────────────────────────
# `command` is the linux binary and darwin overrides it. Paths are relative to
# the bundle root, which is what the spec expects for binary servers.
cat > "$STAGE/manifest.json" <<JSON
{
  "manifest_version": "0.3",
  "name": "marrow",
  "display_name": "Marrow",
  "version": "$VERSION",
  "description": "Shared memory for parallel AI coding agents. Local, free, with rooms and file claims.",
  "long_description": "Marrow gives every coding agent on your machine one brain to write to. Memory survives the end of a session, recall returns linked neighbours rather than only exact matches, and live sessions coordinate through rooms and file claims so two agents never edit the same file at once. Runs locally, stores everything in plain files under .marrow/, and is free under AGPL-3.0.",
  "author": {
    "name": "Mutaqin Aryawijaya",
    "url": "https://github.com/aryawidjaja"
  },
  "homepage": "https://www.marrow.works",
  "documentation": "$REPO#readme",
  "support": "$REPO/issues",
  "repository": {
    "type": "git",
    "url": "$REPO.git"
  },
  "license": "AGPL-3.0-only",
  "keywords": ["memory", "mcp", "agents", "coding", "context"],
  "server": {
    "type": "binary",
    "entry_point": "server/marrow-mcp-linux",
    "mcp_config": {
      "command": "\${__dirname}/server/marrow-mcp-linux",
      "args": ["--root", "\${user_config.project_root}"],
      "env": {},
      "platform_overrides": {
        "darwin": {
          "command": "\${__dirname}/server/marrow-mcp-darwin"
        }
      }
    }
  },
  "user_config": {
    "project_root": {
      "type": "directory",
      "title": "Project directory",
      "description": "The repository Marrow should remember. Its memory lives in .marrow/ inside this folder.",
      "required": true
    }
  },
  "compatibility": {
    "platforms": $PLATFORMS
  }
}
JSON

say "validating manifest"
npx --yes @anthropic-ai/mcpb@latest validate "$STAGE/manifest.json"

say "packing"
npx --yes @anthropic-ai/mcpb@latest pack "$STAGE" "$BUNDLE"

# ── server.json for the MCP registry ────────────────────────────────────────
SHA="$(openssl dgst -sha256 "$BUNDLE" | awk '{print $NF}')"
echo "$SHA  marrow-mcp.mcpb" > "$BUNDLE.sha256"

cat > "$ROOT/server.json" <<JSON
{
  "\$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
  "name": "io.github.aryawidjaja/marrow",
  "title": "Marrow",
  "description": "Shared memory for parallel AI coding agents. Local, free, with rooms and file claims.",
  "repository": {
    "url": "$REPO",
    "source": "github"
  },
  "version": "$VERSION",
  "websiteUrl": "https://www.marrow.works",
  "packages": [
    {
      "registryType": "mcpb",
      "identifier": "$REPO/releases/download/v$VERSION/marrow-mcp.mcpb",
      "version": "$VERSION",
      "fileSha256": "$SHA",
      "transport": {
        "type": "stdio"
      }
    }
  ]
}
JSON

say "done"
printf '  bundle     %s (%s)\n' "$BUNDLE" "$(du -h "$BUNDLE" | cut -f1)"
printf '  sha256     %s\n' "$SHA"
printf '  server.json written for v%s\n' "$VERSION"
