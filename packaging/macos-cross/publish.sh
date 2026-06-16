#!/usr/bin/env bash
# Release publish for the n3o-slic3r macOS app: cross-build + sign the app on
# Linux via osxcross (build.sh — which ad-hoc-signs the .app/.dmg, names the
# .dmg, and GPG-signs it with the project release key), then upload the signed
# .dmg (+ its detached .sig and the public key) so users can verify it with the
# same key the arch / flatpak / windows channels use.
#
# NOTE: the GPG signature is for cross-channel integrity/authenticity
# (`gpg --verify`), NOT Apple notarization. The .app/.dmg is only ad-hoc signed
# (no Developer-ID), so macOS Gatekeeper still shows an "unidentified developer"
# prompt on download — right-click → Open clears it. Notarization needs a paid
# Apple Developer account.
#
# The cross build compiles the *working tree* (cargo), so commit your work first
# for a clean release.
#
# Usage:  publish.sh [arm64|x86_64]      (default: arm64)
#
# Config (env):
#   N3O_GPG_KEY            Signing key fingerprint. Unset → unsigned .dmg (no
#                          default key); set it to GPG-sign.
#   N3O_BASE_URL           Public HTTPS base URL of the site (printed install
#                          commands only); this channel serves from <base>/pkg.
#                          Default: https://n3o.thegraveyard.org
#   N3O_PUBLISH_DEST       Optional rsync/ssh destination *base*, e.g.
#                          user@host:/srv/www/n3o.thegraveyard.org. This channel
#                          uploads to <dest>/pkg (.dmg + signature + public key);
#                          when unset, prints the manual upload steps.
#   OSXCROSS_ROOT          reuse an existing osxcross install; default is the
#                          in-tree build (ensure-osxcross.sh).
#   DMG_TOOL               libdmg-hfsplus `dmg` tool (bundle-app.sh --dmg).
#
# Prereqs: osxcross with a packaged SDK, the cross-deps tree (built on demand
# below when absent), rcodesign (`cargo install apple-codesign`), genisoimage +
# libdmg-hfsplus for the .dmg. See README.md.
set -euo pipefail

arch="${1:-arm64}"
case "$arch" in
  arm64)  triple=aarch64-apple-darwin; label=aarch64 ;;
  x86_64) triple=x86_64-apple-darwin;  label=x64 ;;
  *) echo "arch must be arm64 or x86_64" >&2; exit 2 ;;
esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
source "${repo}/packaging/lib/sign-and-upload.sh"
n3o_signing_init
version="$(grep -m1 '^version' "${repo}/src-tauri/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"

# build.sh is self-contained: it ensures the arch-namespaced cross-deps tree
# (build-deps.sh, the slow one-time step), builds the frontend, cross-builds the
# app, assembles + ad-hoc signs the .app and .dmg, names the .dmg, and GPG-signs
# it.
echo ":: building + signing the macOS app + .dmg (${arch})"
"${here}/build.sh" "${arch}"

# build.sh produced the final, versioned .dmg (GPG-signed when N3O_GPG_KEY is
# set) — just upload it.
dmg="${repo}/target/${triple}/release/bundle/dmg/n3o-slic3r_${version}_${label}.dmg"
[[ -f "${dmg}" ]] || { echo "error: expected ${dmg} after build" >&2; exit 1; }
dmgfile="$(basename "${dmg}")"

n3o_upload "${dmg}" "${dmg}.sig"

cat <<INSTALL

Install on macOS (with signature verification — run gpg on any machine):
  # one-time: import the n3o release public key
  curl -fsSLO ${url}/${keyname}
  gpg --import ${keyname}
  # download the .dmg + its signature, then verify before opening
  curl -fsSLO ${url}/${dmgfile}
  curl -fsSLO ${url}/${dmgfile}.sig
  gpg --verify ${dmgfile}.sig ${dmgfile}
  # on "Good signature" from ${key}: open ${dmgfile}, drag the app to
  # Applications. First launch: right-click the app -> Open (ad-hoc signed,
  # not notarized, so Gatekeeper prompts once).
INSTALL
