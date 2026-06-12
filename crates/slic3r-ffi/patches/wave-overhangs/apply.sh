#!/usr/bin/env bash
# Apply the wave-overhang carry patches onto the pinned OrcaSlicer submodule.
#
# Mirrors packaging/windows-cross/build-deps.sh::patch_orca — idempotent, safe to
# re-run: a patch already applied (or partially) is detected via a reverse-check
# and skipped. A patch that no longer applies is a hard error: the OrcaSlicer pin
# moved and the carry needs re-validating (see README.md "Maintenance").
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
    echo "   The submodule moved; regenerate the carry — see crates/slic3r-ffi/patches/wave-overhangs/README.md" >&2
    exit 1
  fi
done
echo ":: wave-overhang carry applied"
