#!/usr/bin/env bash
# Build the macOS app for an arch from Linux via osxcross: ensure the cross-deps
# tree (build-deps.sh, the slow one-time step), build the frontend, cross-compile
# the app, assemble + ad-hoc-sign the .app and .dmg, give the .dmg its final
# versioned name, and GPG-sign it with the project release key. publish.sh then
# just uploads the result.
#
# Usage:  build.sh <arm64|x86_64>          (default: arm64)
#
# Env: N3O_GPG_KEY (release key, for the GPG signature), OSXCROSS_ROOT (env.sh),
# DMG_TOOL (bundle-app.sh). See README.md.
set -euo pipefail

arch="${1:-arm64}"
case "$arch" in
  arm64)  triple=aarch64-apple-darwin; label=aarch64 ;;
  x86_64) triple=x86_64-apple-darwin;  label=x64 ;;
  *) echo "arch must be arm64 or x86_64" >&2; exit 2 ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
prefix="${repo}/external/OrcaSlicer/deps/build/${arch}/OrcaSlicer_dep/usr/local"

source "${repo}/packaging/lib/sign-and-upload.sh"
n3o_signing_init
version="$(grep -m1 '^version' "${repo}/src-tauri/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"

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

# Give the .dmg its final, versioned, arch-specific name (tauri's native
# convention: n3o-slic3r_<version>_<aarch64|x64>.dmg) so arm64 and x86_64 don't
# collide and users see what they're getting — then GPG-sign it. publish.sh
# just uploads the result.
built_dmg="${repo}/target/${triple}/release/bundle/dmg/n3o-slic3r.dmg"
[[ -f "${built_dmg}" ]] || { echo "error: no .dmg at ${built_dmg} after bundle" >&2; exit 1; }
dmg="${repo}/target/${triple}/release/bundle/dmg/n3o-slic3r_${version}_${label}.dmg"
mv -f "${built_dmg}" "${dmg}"
n3o_sign "${dmg}" dmg
