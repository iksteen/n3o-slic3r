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
#   N3O_ARCH_GPG_KEY       Signing key fingerprint. Defaults to the same
#                          dedicated project key the flatpak uses.
#   N3O_ARCH_URL           Public HTTPS base URL the package is served
#                          from (used only for the printed install
#                          commands). Default: https://n3o.thegraveyard.org/pkg
#   N3O_ARCH_PUBLISH_DEST  Optional rsync/ssh destination, e.g.
#                          user@host:/srv/www/n3o.thegraveyard.org/pkg.
#                          When set, the script uploads the package +
#                          signature + public key there; when unset it
#                          prints the manual steps.
#
# Build deps: the PKGBUILD's makedepends (rust, nodejs, npm, cmake,
# ninja, git) must be installed — `makepkg -s` will pull missing ones
# via pacman (sudo). See packaging/flatpak/PUBLISHING.md for the shared
# signing-key setup; the public key is committed at
# packaging/flatpak/n3o-slic3r-signing-key.asc.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"

# Same dedicated release key as the flatpak channel. Override to sign
# with a different key.
key="${N3O_ARCH_GPG_KEY:-B3D305B467D790E9328FFDF3D0B98FE70335DC53}"
url="${N3O_ARCH_URL:-https://n3o.thegraveyard.org/pkg}"
url="${url%/}"
keyfile="${repo}/packaging/flatpak/n3o-slic3r-signing-key.asc"
keyname="$(basename "${keyfile}")"

cd "${here}"

echo ":: makepkg build + GPG sign (key ${key})"
# -s: install missing makedepends; -f: rebuild even if a package exists;
# --sign --key: produce a detached `${pkg}.sig` with the project key.
makepkg -sf --sign --key "${key}"

# Resolve the exact built artifact (honors a maintainer's PKGDEST /
# PKGEXT instead of assuming the filename).
pkg="$(makepkg --packagelist | head -n1)"
sig="${pkg}.sig"
if [[ ! -f "${pkg}" || ! -f "${sig}" ]]; then
  echo "error: expected a signed package at ${pkg} (+ ${sig}) after build" >&2
  exit 1
fi
pkgfile="$(basename "${pkg}")"

echo
echo "Built + signed:"
echo "  package:   ${pkg}"
echo "  signature: ${sig}"

if [[ -n "${N3O_ARCH_PUBLISH_DEST:-}" ]]; then
  dest="${N3O_ARCH_PUBLISH_DEST%/}"
  echo
  echo ":: uploading to ${dest}/ (N3O_ARCH_PUBLISH_DEST set)"
  # The package + its detached signature (pacman -U fetches `<url>.sig`
  # alongside) + the public key so users can import and trust it.
  rsync -a "${pkg}" "${sig}" "${dest}/"
  [[ -f "${keyfile}" ]] && rsync -a "${keyfile}" "${dest}/"
  echo ":: uploaded."
else
  cat <<DONE

Set N3O_ARCH_PUBLISH_DEST=<rsync/ssh dest> (e.g.
user@host:/srv/www/n3o.thegraveyard.org/pkg) to upload automatically,
or by hand:
  rsync -a "${pkg}" "${sig}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
  rsync -a "${keyfile}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
DONE
fi

cat <<INSTALL

Install on a clean Arch machine (signed):
  # one-time: import + locally trust the n3o release key
  curl -fsSLO ${url}/${keyname}
  sudo pacman-key --add ${keyname}
  sudo pacman-key --lsign-key ${key}
  # then install (pacman fetches and verifies the .sig automatically)
  sudo pacman -U ${url}/${pkgfile}
INSTALL
