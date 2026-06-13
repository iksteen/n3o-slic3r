#!/usr/bin/env bash
# Remove the macOS cross-build artifacts: the cross scratch (.build — sources,
# per-dep builds, libdmg-hfsplus, run logs), both arch dep trees, the
# apple-darwin cargo targets, and the FFI cmake build dirs (+ the
# slic3r-ffi-current symlink). Shared workspace artifacts belong to
# scripts/clean.sh.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

dirs=(
  "$here/.build"
  "$root/external/OrcaSlicer/deps/build/arm64"
  "$root/external/OrcaSlicer/deps/build/x86_64"
  "$root/target/aarch64-apple-darwin"
  "$root/target/x86_64-apple-darwin"
  "$root/build/slic3r-ffi-arm64"
  "$root/build/slic3r-ffi-x86_64"
)
for d in "${dirs[@]}"; do
  if [ -e "$d" ]; then echo ":: rm -rf ${d#"$root"/}"; rm -rf -- "$d"; fi
done

# The slic3r-ffi-current symlink (build.rs points it at the mac arch it built).
if [ -L "$root/build/slic3r-ffi-current" ]; then
  echo ":: rm build/slic3r-ffi-current"; rm -f "$root/build/slic3r-ffi-current"
fi
# Run logs alongside .build (e.g. .build-arm64.log).
shopt -s nullglob
for f in "$here"/.build-*.log; do echo ":: rm ${f#"$root"/}"; rm -f -- "$f"; done
shopt -u nullglob

echo ":: macos-cross clean complete."
