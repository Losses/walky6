#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
VENDOR_DIR="$ROOT/vendor/sneakerweb"
PATCH_FILE="$ROOT/patches/sneakerweb.patch"
UPSTREAM_URL="https://codeberg.org/worm-blossom/sneakerweb"
UPSTREAM_REV="888cf132207a2bf0622a5633a2d347e9e910538c"

if [ -d "$VENDOR_DIR/src" ]; then
  echo "[vendor] sneakerweb already vendored"
  exit 0
fi

echo "[vendor] cloning $UPSTREAM_URL @ $UPSTREAM_REV ..."
git clone "$UPSTREAM_URL" "$VENDOR_DIR"
git -C "$VENDOR_DIR" checkout "$UPSTREAM_REV"

echo "[vendor] applying patches ..."
git -C "$VENDOR_DIR" apply "$PATCH_FILE"

echo "[vendor] done"
