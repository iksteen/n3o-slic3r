#!/usr/bin/env bash
# Apply the wipe-tower-pa carry patch onto the pinned OrcaSlicer submodule.
#
# Mirrors packaging/windows-cross/build-deps.sh::patch_orca — idempotent, safe to
# re-run: a fully-applied patch is detected via a reverse-check and skipped. A
# patch that no longer applies cleanly (moved pin, or a partial apply) is a hard
# error — the carry needs re-validating (see README.md "Maintenance").
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
orca="$here/../../../../external/OrcaSlicer"

for p in "$here"/patches/*.patch; do
  name="$(basename "$p")"
  if git -C "$orca" apply --check "$p" 2>/dev/null; then
    git -C "$orca" apply "$p"
    echo ":: applied $name"
  elif git -C "$orca" apply --reverse --check "$p" 2>/dev/null; then
    echo ":: $name already applied"
  else
    echo "!! $name does NOT apply cleanly to the current OrcaSlicer pin." >&2
    echo "   The submodule moved; regenerate the carry — see crates/slic3r-ffi/patches/wipe-tower-pa/README.md" >&2
    exit 1
  fi
done
echo ":: wipe-tower-pa carry applied"
