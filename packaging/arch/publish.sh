#!/usr/bin/env bash
# Release publish for the n3o-slic3r Arch package: build with makepkg,
# GPG-sign with the project release key, and upload the signed
# `.pkg.tar.zst` so users can install it with a bare `pacman -U <url>`.
#
# This is NOT a full pacman repo — just a single signed package file
# (plus its detached `.sig` and the public key) served over HTTPS. The
# flatpak path (packaging/flatpak/publish.sh) is the parallel channel;
# both sign with the same dedicated project key.
#
# Config (env):
#   N3O_GPG_KEY       Signing key fingerprint. Defaults to the shared
#                     dedicated project release key (same across all channels).
#   N3O_BASE_URL      Public HTTPS base URL of the site (used only for the
#                     printed install commands); this channel serves from
#                     <base>/pkg. Default: https://n3o.thegraveyard.org
#   N3O_PUBLISH_DEST  Optional rsync/ssh destination *base*, e.g.
#                     user@host:/srv/www/n3o.thegraveyard.org. This channel
#                     uploads to <dest>/pkg (package + signature + public
#                     key); when unset it prints the manual steps.
#
# Build deps: the PKGBUILD's makedepends (rust, nodejs, npm, cmake,
# ninja, git) must be installed — `makepkg -s` will pull missing ones
# via pacman (sudo). See packaging/flatpak/PUBLISHING.md for the shared
# signing-key setup; the public key is committed at
# packaging/flatpak/n3o-slic3r-signing-key.asc.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
source "${repo}/packaging/lib/sign-and-upload.sh"
n3o_signing_init

echo ":: build (makepkg, unsigned)"
"${here}/build.sh"

# Resolve the exact built artifact (honors a maintainer's PKGDEST / PKGEXT
# instead of assuming the filename); --packagelist needs the PKGBUILD dir.
pkg="$(cd "${here}" && makepkg --packagelist | head -n1)"
if [[ ! -f "${pkg}" ]]; then
  echo "error: expected a built package at ${pkg} after build" >&2
  exit 1
fi
pkgfile="$(basename "${pkg}")"

n3o_sign_and_upload "${pkg}" package

cat <<INSTALL

Install on a clean Arch machine (signed):
  # one-time: import + locally trust the n3o release key
  curl -fsSLO ${url}/${keyname}
  sudo pacman-key --add ${keyname}
  sudo pacman-key --lsign-key ${key}
  # then install (pacman fetches and verifies the .sig automatically)
  sudo pacman -U ${url}/${pkgfile}
INSTALL
