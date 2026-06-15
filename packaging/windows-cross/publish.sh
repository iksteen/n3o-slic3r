#!/usr/bin/env bash
# Release publish for the n3o-slic3r Windows installer: cross-build the NSIS
# installer on Linux (build-app.sh), GPG-sign it with the project release key,
# and upload the signed `-setup.exe` (+ its detached `.sig` and the public key)
# so users can verify it with the same key the arch and flatpak channels use.
#
# The arch (packaging/arch/publish.sh) and flatpak (packaging/flatpak/
# publish.sh) paths are the parallel channels; all three sign with the same
# dedicated project key.
#
# NOTE: this is GPG signing for cross-channel integrity/authenticity
# verification, NOT Windows Authenticode code-signing. Without an Authenticode
# cert (wired via tauri's `bundle.windows.signCommand`), Windows SmartScreen
# still shows an "unknown publisher" prompt — the `.sig` is for `gpg --verify`,
# not the Windows trust UI.
#
# The cross build compiles the *working tree* (cargo), so commit your work
# first for a clean release.
#
# Config (env):
#   WINCROSS_PREFIX        Cross-deps prefix from build-deps.sh. build-app.sh
#                          defaults it to packaging/windows-cross/.build/prefix.
#   N3O_GPG_KEY            Signing key fingerprint. Defaults to the shared
#                          dedicated project release key (same across channels).
#   N3O_BASE_URL           Public HTTPS base URL of the site (printed install
#                          commands only); this channel serves from <base>/pkg.
#                          Default: https://n3o.thegraveyard.org
#   N3O_PUBLISH_DEST       Optional rsync/ssh destination *base*, e.g.
#                          user@host:/srv/www/n3o.thegraveyard.org. This channel
#                          uploads to <dest>/pkg (installer + signature + public
#                          key); when unset, prints the manual steps.
#
# Prereqs: cargo-xwin, the x86_64-pc-windows-msvc rust target, and node deps.
# The cross-deps prefix is built on demand (build-deps.sh) when absent, then
# reused; see WINCROSS_PREFIX above and build-app.sh.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
source "${repo}/packaging/lib/sign-and-upload.sh"
n3o_signing_init
target="x86_64-pc-windows-msvc"

# build.sh is self-contained: it ensures the cross-deps tree (build-deps.sh, the
# slow one-time step) and cross-builds the app + NSIS installer.
echo ":: cross-building the Windows app + NSIS installer"
"${here}/build.sh"

# Resolve the produced installer (the version is in the filename, so glob it
# rather than assuming).
bundle_dir="${repo}/target/${target}/release/bundle/nsis"
setup="$(ls -1 "${bundle_dir}"/*-setup.exe 2>/dev/null | head -n1 || true)"
if [[ -z "${setup}" || ! -f "${setup}" ]]; then
  echo "error: no *-setup.exe in ${bundle_dir} after build" >&2
  exit 1
fi
setupfile="$(basename "${setup}")"

n3o_sign_and_upload "${setup}" installer

cat <<INSTALL

Install on Windows (with signature verification — run the gpg steps on any
machine that has gpg; PowerShell/curl shown):
  # one-time: import the n3o release public key
  curl -fsSLO ${url}/${keyname}
  gpg --import ${keyname}
  # download the installer + its signature, then verify before running
  curl -fsSLO ${url}/${setupfile}
  curl -fsSLO ${url}/${setupfile}.sig
  gpg --verify ${setupfile}.sig ${setupfile}
  # on "Good signature" from ${key}, run ${setupfile}
INSTALL
