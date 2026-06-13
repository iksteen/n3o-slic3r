#!/usr/bin/env bash
# Build the n3o-slic3r Arch package (unsigned) with makepkg. Produces the
# `*.pkg.tar.zst` in this directory; publish.sh GPG-signs + uploads it.
#
# `-s` installs missing makedepends via pacman (sudo); `-f` rebuilds even if a
# package already exists. Extra args pass through to makepkg.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${here}"
exec makepkg -sf "$@"
