#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
# 1. Vendor sneakerweb
VENDOR_DIR_SNEAKERWEB="$ROOT/vendor/sneakerweb"
PATCH_FILE_SNEAKERWEB="$ROOT/patches/sneakerweb.patch"
UPSTREAM_URL_SNEAKERWEB="https://codeberg.org/worm-blossom/sneakerweb"
UPSTREAM_REV_SNEAKERWEB="888cf132207a2bf0622a5633a2d347e9e910538c"

if [ ! -d "$VENDOR_DIR_SNEAKERWEB/src" ]; then
  echo "[vendor] cloning sneakerweb @ $UPSTREAM_REV_SNEAKERWEB ..."
  git clone "$UPSTREAM_URL_SNEAKERWEB" "$VENDOR_DIR_SNEAKERWEB"
  git -c advice.detachedHead=false -C "$VENDOR_DIR_SNEAKERWEB" checkout "$UPSTREAM_REV_SNEAKERWEB"

  echo "[vendor] applying sneakerweb patches ..."
  git -C "$VENDOR_DIR_SNEAKERWEB" apply "$PATCH_FILE_SNEAKERWEB"
else
  echo "[vendor] sneakerweb already vendored"
fi

# 2. Vendor bab_rs
VENDOR_DIR_BAB_RS="$ROOT/vendor/bab_rs"
PATCH_FILE_BAB_RS="$ROOT/patches/bab_rs.patch"
UPSTREAM_URL_BAB_RS="https://codeberg.org/worm-blossom/bab_rs"
UPSTREAM_REV_BAB_RS="2dd7466083424eccdecc1c2f43a36fef7acc8a83"

if [ ! -d "$VENDOR_DIR_BAB_RS/src" ]; then
  echo "[vendor] cloning bab_rs @ $UPSTREAM_REV_BAB_RS ..."
  git clone "$UPSTREAM_URL_BAB_RS" "$VENDOR_DIR_BAB_RS"
  git -c advice.detachedHead=false -C "$VENDOR_DIR_BAB_RS" checkout "$UPSTREAM_REV_BAB_RS"

  echo "[vendor] applying bab_rs patches ..."
  git -C "$VENDOR_DIR_BAB_RS" apply "$PATCH_FILE_BAB_RS"
else
  echo "[vendor] bab_rs already vendored"
fi

# 3. Vendor willow25
VENDOR_DIR_WILLOW25="$ROOT/vendor/willow25"
PATCH_FILE_WILLOW25="$ROOT/patches/willow25.patch"
UPSTREAM_URL_WILLOW25="https://codeberg.org/worm-blossom/willow_rs"
UPSTREAM_REV_WILLOW25="17b1a057c35a0da3710fdebb57804fad4a19cc3c"

if [ ! -d "$VENDOR_DIR_WILLOW25/willow25/src" ]; then
  echo "[vendor] cloning willow25 @ $UPSTREAM_REV_WILLOW25 ..."
  git clone "$UPSTREAM_URL_WILLOW25" "$VENDOR_DIR_WILLOW25"
  git -c advice.detachedHead=false -C "$VENDOR_DIR_WILLOW25" checkout "$UPSTREAM_REV_WILLOW25"

  echo "[vendor] applying willow25 patches ..."
  git -C "$VENDOR_DIR_WILLOW25" apply "$PATCH_FILE_WILLOW25"
else
  echo "[vendor] willow25 already vendored"
fi

echo "[vendor] done"
