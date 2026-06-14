#!/usr/bin/env bash
# Release publish for the n3o-slic3r macOS app: cross-build the app on Linux via
# osxcross (build.sh), assemble + ad-hoc-sign the .app and a .dmg (bundle-app.sh),
# GPG-sign the .dmg with the project release key, and upload the signed .dmg (+
# its detached .sig and the public key) so users can verify it with the same key
# the arch / flatpak / windows channels use.
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
#   N3O_GPG_KEY            Signing key fingerprint. Defaults to the shared
#                          dedicated project release key (same across channels).
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

# Shared dedicated release key (same across all channels). Override to sign
# with a different key.
key="${N3O_GPG_KEY:-B3D305B467D790E9328FFDF3D0B98FE70335DC53}"
# Single base URL for the whole site; this channel serves from <base>/pkg.
base_url="${N3O_BASE_URL:-https://n3o.thegraveyard.org}"; url="${base_url%/}/pkg"
keyfile="${repo}/packaging/flatpak/n3o-slic3r-signing-key.asc"
keyname="$(basename "${keyfile}")"
version="$(grep -m1 '^version' "${repo}/src-tauri/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')"

# build.sh is self-contained: it ensures the arch-namespaced cross-deps tree
# (build-deps.sh, the slow one-time step), builds the frontend, cross-builds the
# app, and assembles + ad-hoc signs the .app and .dmg.
echo ":: building the macOS app + .dmg (${arch})"
"${here}/build.sh" "${arch}"

# Give the published artifact a versioned, arch-specific name (tauri's native
# convention: n3o-slic3r_<version>_<aarch64|x64>.dmg) so arm64 and x86_64 don't
# collide on the server and users see what they're getting.
built_dmg="${repo}/target/${triple}/release/bundle/dmg/n3o-slic3r.dmg"
[[ -f "${built_dmg}" ]] || { echo "error: no .dmg at ${built_dmg} after bundle" >&2; exit 1; }
dmg="${repo}/target/${triple}/release/bundle/dmg/n3o-slic3r_${version}_${label}.dmg"
cp -f "${built_dmg}" "${dmg}"
dmgfile="$(basename "${dmg}")"

echo ":: GPG sign ${dmgfile} (key ${key})"
gpg --batch --yes --local-user "${key}" --detach-sign "${dmg}"
sig="${dmg}.sig"
[[ -f "${sig}" ]] || { echo "error: signing did not produce ${sig}" >&2; exit 1; }

echo
echo "Built + signed:"
echo "  dmg:       ${dmg}"
echo "  signature: ${sig}"

if [[ -n "${N3O_PUBLISH_DEST:-}" ]]; then
  dest="${N3O_PUBLISH_DEST%/}/pkg"
  echo
  echo ":: uploading to ${dest}/ (N3O_PUBLISH_DEST set)"
  rsync -a "${dmg}" "${sig}" "${dest}/"
  [[ -f "${keyfile}" ]] && rsync -a "${keyfile}" "${dest}/"
  echo ":: uploaded."
else
  cat <<DONE

Set N3O_PUBLISH_DEST=<rsync/ssh dest base> (e.g.
user@host:/srv/www/n3o.thegraveyard.org) to upload automatically (this channel
uploads to <dest>/pkg), or by hand:
  rsync -a "${dmg}" "${sig}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
  rsync -a "${keyfile}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
DONE
fi

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
