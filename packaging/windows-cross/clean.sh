#!/usr/bin/env bash
# Remove the Windows cross-build artifacts: the cross-deps scratch + prefix
# (.build), the windows cargo target dir(s), and the FFI cmake build dir. Shared
# workspace artifacts belong to scripts/clean.sh.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

paths=("$here/.build" "$root/build/slic3r-ffi-win")
shopt -s nullglob
paths+=("$root"/target/x86_64-pc-windows-*)
shopt -u nullglob

for p in "${paths[@]}"; do
  if [ -e "$p" ]; then echo ":: rm -rf ${p#"$root"/}"; rm -rf -- "$p"; fi
done

echo ":: windows-cross clean complete."
