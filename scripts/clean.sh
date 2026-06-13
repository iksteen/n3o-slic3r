#!/usr/bin/env bash
# Remove ALL build artifacts so a subsequent build (`npm run publish:all`, a
# plain `cargo build`, etc.) rebuilds absolutely everything from scratch:
#   - cargo output (every target, incl. the Linux + cross trees)
#   - the slic3r-ffi cmake build dirs (+ the slic3r-ffi-current symlink)
#   - the vite frontend bundle (rebuilt by `npm run build`)
#   - the OrcaSlicer dependency trees (native + macOS arm64/x86_64 cross)
#   - each release channel's packaging scratch (macos/windows cross, flatpak,
#     arch makepkg outputs)
#
# Does NOT touch: node_modules (an install, not a build artifact — re-add with
# `npm ci`), the git checkout, or the OrcaSlicer submodule working tree (its
# build-time patches re-apply idempotently). The downloaded dep-source caches
# under external/OrcaSlicer/deps/DL_CACHE are also left (re-downloading sources
# isn't rebuilding — delete that dir by hand for a truly cold start).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Whole directories.
dirs=(
  target
  build
  dist
  external/OrcaSlicer/deps/build
  external/OrcaSlicer/build
  packaging/macos-cross/.build
  packaging/windows-cross/.build
  packaging/flatpak/.build
  packaging/flatpak/.publish-repo
  packaging/flatpak/.gen
  packaging/flatpak/.flatpak-builder
  packaging/arch/src
  packaging/arch/pkg
)
for d in "${dirs[@]}"; do
  if [ -e "$d" ]; then echo ":: rm -rf $d"; rm -rf -- "$d"; fi
done

# Globbed leftovers (signed packages + run logs). nullglob so an unmatched
# pattern expands to nothing rather than the literal glob.
shopt -s nullglob
for f in \
  packaging/arch/*.pkg.tar.* \
  packaging/macos-cross/.build-*.log
do
  echo ":: rm $f"; rm -f -- "$f"
done
shopt -u nullglob

echo ":: clean complete — the next build rebuilds everything from scratch."
