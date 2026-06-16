#!/usr/bin/env bash
# Build the n3o-slic3r Arch package with makepkg and GPG-sign it (when
# N3O_GPG_KEY is set). Produces the `*.pkg.tar.zst` (+ `.sig`) in this
# directory; publish.sh just uploads the result.
#
# `-s` installs missing makedepends via pacman (sudo); `-f` rebuilds even if a
# package already exists. Extra args pass through to makepkg.
#
# Env: N3O_GPG_KEY (release key; unset → unsigned). See README.md.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
source "${repo}/packaging/lib/sign-and-upload.sh"
n3o_signing_init

echo ":: build (makepkg)"
( cd "${here}" && makepkg -sf "$@" )

# Resolve the exact built artifact (honors a maintainer's PKGDEST / PKGEXT
# instead of assuming the filename); --packagelist needs the PKGBUILD dir.
pkg="$(cd "${here}" && makepkg --packagelist | head -n1)"
[[ -f "${pkg}" ]] || { echo "error: expected a built package at ${pkg} after build" >&2; exit 1; }

n3o_sign "${pkg}" package
