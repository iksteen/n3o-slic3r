#!/usr/bin/env bash
# Remove the flatpak packaging artifacts: flatpak-builder's state/cache
# (.flatpak-builder — incremental module layers, ccache, downloads), the build
# tree (.build), the dev + publish ostree repos (.repo, .publish-repo), and the
# generated manifest/ref (.gen). Shared workspace artifacts belong to
# scripts/clean.sh.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${here}"

for d in .build .repo .publish-repo .gen .flatpak-builder; do
  if [ -e "$d" ]; then echo ":: rm -rf packaging/flatpak/$d"; rm -rf -- "$d"; fi
done

echo ":: flatpak clean complete."
