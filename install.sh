#!/bin/sh
# Install Marrow's prebuilt binaries — no Rust toolchain required.
#
#   curl -fsSL https://raw.githubusercontent.com/aryawidjaja/marrow/main/install.sh | sh
#
# Installs `marrow`, `marrow-mcp`, `marrow-serve`, and `marrow-server` into ~/.local/bin
# (override the destination with MARROW_BIN_DIR, or the version with MARROW_VERSION).
set -eu

repo="aryawidjaja/marrow"
bindir="${MARROW_BIN_DIR:-$HOME/.local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
  Darwin/arm64)  target="aarch64-apple-darwin" ;;
  Darwin/x86_64) target="x86_64-apple-darwin" ;;
  Linux/x86_64)  target="x86_64-unknown-linux-gnu" ;;
  *)
    echo "No prebuilt binary for $os/$arch. Install with Rust instead:"
    echo "  cargo install --git https://github.com/$repo marrow-cli marrow-mcp marrow-web marrow-server"
    exit 1 ;;
esac

tag="${MARROW_VERSION:-$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" \
  | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)}"
if [ -z "$tag" ]; then
  echo "No published release found yet. Try again shortly, or install with Rust:"
  echo "  cargo install --git https://github.com/$repo marrow-cli marrow-mcp marrow-web marrow-server"
  exit 1
fi

asset="marrow-$tag-$target.tar.gz"
url="https://github.com/$repo/releases/download/$tag/$asset"
echo "Downloading $url"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$asset"
if ! curl -fsSL "$url.sha256" -o "$tmp/$asset.sha256"; then
  if [ "${MARROW_ALLOW_UNVERIFIED:-0}" != "1" ]; then
    echo "This release has no checksum, so the installer will not run it." >&2
    echo "Choose a newer release or set MARROW_ALLOW_UNVERIFIED=1 for a legacy release." >&2
    exit 1
  fi
  echo "Warning: installing a legacy release without verification." >&2
else
  expected="$(sed -n '1{s/[[:space:]].*//;p;}' "$tmp/$asset.sha256")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$tmp/$asset" | sed 's/[[:space:]].*//')"
  else
    actual="$(shasum -a 256 "$tmp/$asset" | sed 's/[[:space:]].*//')"
  fi
  if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
    echo "Checksum verification failed for $asset." >&2
    exit 1
  fi
fi
tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$bindir"
for b in marrow marrow-mcp marrow-serve marrow-server; do
  src="$(find "$tmp" -type f -name "$b" | head -n1)"
  [ -n "$src" ] && install -m 0755 "$src" "$bindir/$b"
done

echo "Installed marrow, marrow-mcp, marrow-serve, marrow-server to $bindir"
case ":$PATH:" in
  *":$bindir:"*) ;;
  *) echo "Add it to your PATH:  export PATH=\"$bindir:\$PATH\"" ;;
esac
cat <<'NEXT'

Next steps:
  cd your-project
  marrow setup            # wire this project into Claude Code (add --global for every project)
  # then restart Claude Code

Onboarding an existing repo? Run `marrow ingest` (or ask your agent to "seed marrow from this
repo's docs"). Capture a session anytime with /marrow-save.

Search is keyword by default. For smarter meaning-based recall, enable semantic search (opt-in,
needs an embedding model) — see `marrow embed` and the README.

Docs: https://github.com/aryawidjaja/marrow
NEXT
