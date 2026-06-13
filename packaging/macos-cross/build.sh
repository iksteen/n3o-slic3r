#!/usr/bin/env bash
# Build the macOS app for an arch from Linux via osxcross: ensure the cross-deps
# tree (build-deps.sh, the slow one-time step), build the frontend, cross-compile
# the app, and assemble + ad-hoc-sign the .app and .dmg. publish.sh then GPG-signs
# + uploads the .dmg.
#
# Usage:  build.sh <arm64|x86_64>          (default: arm64)
#
# Env: OSXCROSS_ROOT (env.sh), DMG_TOOL (bundle-app.sh). See README.md.
set -euo pipefail

arch="${1:-arm64}"
case "$arch" in
  arm64)  triple=aarch64-apple-darwin ;;
  x86_64) triple=x86_64-apple-darwin ;;
  *) echo "arch must be arm64 or x86_64" >&2; exit 2 ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
prefix="${repo}/external/OrcaSlicer/deps/build/${arch}/OrcaSlicer_dep/usr/local"

# Ensure the arch-namespaced cross-deps tree. build-deps.sh is the slow one-time
# step; reuse only when *complete* (the .deps-complete stamp), not when a
# partial/interrupted run left early deps behind.
if [[ -f "${prefix}/.deps-complete" ]]; then
  echo ":: reusing complete cross-deps prefix at ${prefix}"
else
  echo ":: cross-deps prefix for ${arch} missing or incomplete — building it (one-time, slow)"
  "${here}/build-deps.sh" "${arch}"
fi

# Build the frontend bundle (dist/). The raw cargo build below — unlike
# `tauri build` — does NOT run tauri.conf.json's beforeBuildCommand, so build it
# here (ships the current UI; self-contained after `npm run clean`).
echo ":: building the frontend (npm run build)"
( cd "${repo}" && npm run build )

# --features custom-protocol is REQUIRED: without it Tauri builds a dev-mode
# binary that loads the dev server (white screen) instead of the embedded UI.
echo ":: cross-building the macOS app (${arch})"
"${here}/env.sh" "${arch}" cargo build -p n3o-slic3r --target "${triple}" --release --features custom-protocol

echo ":: assembling + ad-hoc signing the .app and .dmg"
"${here}/bundle-app.sh" "${arch}" --dmg
