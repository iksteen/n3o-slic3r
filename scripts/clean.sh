#!/usr/bin/env bash
# Remove ALL build artifacts so the next build rebuilds everything from scratch.
# Composes each packaging channel's own clean.sh (their scratch + arch-specific
# outputs), then sweeps the shared/native remainder: cargo output (every target),
# the slic3r-ffi cmake build dirs, the vite bundle, and the OrcaSlicer dep trees.
#
# Does NOT touch: node_modules (an install, not a build artifact — re-add with
# `npm ci`), the git checkout, the OrcaSlicer submodule working tree (its
# build-time patches re-apply idempotently), or the downloaded dep-source caches
# under external/OrcaSlicer/deps/DL_CACHE (re-downloading sources isn't
# rebuilding — delete that dir by hand for a truly cold start).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Per-channel cleans (packaging scratch + each channel's arch-specific outputs).
for c in arch flatpak windows-cross macos-cross; do
  if [ -x "packaging/$c/clean.sh" ]; then
    echo "== packaging/$c =="; "packaging/$c/clean.sh"
  fi
done

# Shared / native remainder not owned by a single channel.
echo "== shared =="
for d in target build dist external/OrcaSlicer/deps/build external/OrcaSlicer/build; do
  if [ -e "$d" ]; then echo ":: rm -rf $d"; rm -rf -- "$d"; fi
done

echo ":: clean complete — the next build rebuilds everything from scratch."
