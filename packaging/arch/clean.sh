#!/usr/bin/env bash
# Remove the Arch packaging artifacts: makepkg's src/ + pkg/ scratch and the
# built (+ signed) package files. Shared workspace artifacts (cargo target/, the
# OrcaSlicer deps tree) belong to scripts/clean.sh, not here.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${here}"

for d in src pkg; do
  if [ -e "$d" ]; then echo ":: rm -rf packaging/arch/$d"; rm -rf -- "$d"; fi
done
shopt -s nullglob
for f in *.pkg.tar.*; do echo ":: rm packaging/arch/$f"; rm -f -- "$f"; done
shopt -u nullglob

echo ":: arch clean complete."
