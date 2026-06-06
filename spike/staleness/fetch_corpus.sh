#!/usr/bin/env bash
# Vendors the rich library at a pinned commit into corpus/.
# Pinned for reproducibility — do not bump without re-running the whole spike.
set -euo pipefail
PIN="v13.7.1"
DEST="$(dirname "$0")/corpus"
rm -rf "$DEST"
git clone --depth 1 --branch "$PIN" https://github.com/Textualize/rich.git "$DEST"
# We only need the importable source tree.
echo "Vendored rich@$PIN into $DEST"
git -C "$DEST" rev-parse HEAD > "$DEST/.pinned-commit"
