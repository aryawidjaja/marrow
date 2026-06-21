#!/bin/sh
# Install Marrow's prebuilt binaries — no Rust toolchain required.
#
#   curl -fsSL https://raw.githubusercontent.com/aryawidjaja/marrow/main/install.sh | sh
#
# Installs `marrow`, `marrow-mcp`, and `marrow-serve` into ~/.local/bin
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
    echo "  cargo install --git https://github.com/$repo marrow-cli marrow-mcp"
    exit 1 ;;
esac

tag="${MARROW_VERSION:-$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" \
  | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)}"
if [ -z "$tag" ]; then
  echo "No published release found yet. Try again shortly, or install with Rust:"
  echo "  cargo install --git https://github.com/$repo marrow-cli marrow-mcp"
  exit 1
fi

asset="marrow-$tag-$target.tar.gz"
url="https://github.com/$repo/releases/download/$tag/$asset"
echo "Downloading $url"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$asset"
tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$bindir"
for b in marrow marrow-mcp marrow-serve; do
  src="$(find "$tmp" -type f -name "$b" | head -n1)"
  [ -n "$src" ] && install -m 0755 "$src" "$bindir/$b"
done

echo "Installed marrow, marrow-mcp, marrow-serve to $bindir"
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
repo's docs"). Capture a session anytime with /marrow-save. Docs: https://github.com/aryawidjaja/marrow
NEXT
