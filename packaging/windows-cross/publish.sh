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
#   N3O_WIN_GPG_KEY        Signing key fingerprint. Defaults to the same
#                          dedicated project key the arch/flatpak channels use.
#   N3O_WIN_URL            Public HTTPS base URL the installer is served from
#                          (used only for the printed install commands).
#                          Default: https://n3o.thegraveyard.org/windows
#   N3O_WIN_PUBLISH_DEST   Optional rsync/ssh destination, e.g.
#                          user@host:/srv/www/n3o.thegraveyard.org/windows.
#                          When set, uploads the installer + signature + public
#                          key; when unset, prints the manual steps.
#
# Prereqs: cargo-xwin, the x86_64-pc-windows-msvc rust target, and node deps.
# The cross-deps prefix is built on demand (build-deps.sh) when absent, then
# reused; see WINCROSS_PREFIX above and build-app.sh.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"

# Same dedicated release key as the arch/flatpak channels. Override to sign
# with a different key.
key="${N3O_WIN_GPG_KEY:-B3D305B467D790E9328FFDF3D0B98FE70335DC53}"
url="${N3O_WIN_URL:-https://n3o.thegraveyard.org/windows}"
url="${url%/}"
keyfile="${repo}/packaging/flatpak/n3o-slic3r-signing-key.asc"
keyname="$(basename "${keyfile}")"
target="x86_64-pc-windows-msvc"

# Self-contained like the arch (makepkg -s) and flatpak (orca-deps module)
# publish paths: ensure the cross-deps prefix before build-app.sh, which
# otherwise hard-errors on a missing prefix. build-deps.sh is the slow one-time
# step (the whole libslic3r dep tree); reuse the prefix only when it's *complete*
# — gate on the .deps-complete stamp build-deps.sh writes at the end, not just on
# lib/ existing (a partial/interrupted run leaves early deps' libs behind).
prefix="${WINCROSS_PREFIX:-${here}/.build/prefix}"
if [[ -f "${prefix}/.deps-complete" ]]; then
  echo ":: reusing complete cross-deps prefix at ${prefix} (rm it or run build-deps.sh to rebuild)"
else
  echo ":: cross-deps prefix missing or incomplete — building it (one-time, slow)"
  "${here}/build-deps.sh"
fi

echo ":: cross-building the Windows app + NSIS installer"
"${here}/build-app.sh"

# Resolve the produced installer (the version is in the filename, so glob it
# rather than assuming).
bundle_dir="${repo}/target/${target}/release/bundle/nsis"
setup="$(ls -1 "${bundle_dir}"/*-setup.exe 2>/dev/null | head -n1 || true)"
if [[ -z "${setup}" || ! -f "${setup}" ]]; then
  echo "error: no *-setup.exe in ${bundle_dir} after build" >&2
  exit 1
fi

echo ":: GPG sign $(basename "${setup}") (key ${key})"
# Detached signature next to the installer; users verify with `gpg --verify`.
gpg --batch --yes --local-user "${key}" --detach-sign "${setup}"
sig="${setup}.sig"
if [[ ! -f "${sig}" ]]; then
  echo "error: signing did not produce ${sig}" >&2
  exit 1
fi
setupfile="$(basename "${setup}")"

echo
echo "Built + signed:"
echo "  installer: ${setup}"
echo "  signature: ${sig}"

if [[ -n "${N3O_WIN_PUBLISH_DEST:-}" ]]; then
  dest="${N3O_WIN_PUBLISH_DEST%/}"
  echo
  echo ":: uploading to ${dest}/ (N3O_WIN_PUBLISH_DEST set)"
  # The installer + its detached signature + the public key so users can
  # import, trust, and verify it.
  rsync -a "${setup}" "${sig}" "${dest}/"
  [[ -f "${keyfile}" ]] && rsync -a "${keyfile}" "${dest}/"
  echo ":: uploaded."
else
  cat <<DONE

Set N3O_WIN_PUBLISH_DEST=<rsync/ssh dest> (e.g.
user@host:/srv/www/n3o.thegraveyard.org/windows) to upload automatically,
or by hand:
  rsync -a "${setup}" "${sig}" your-server:/srv/www/n3o.thegraveyard.org/windows/
  rsync -a "${keyfile}" your-server:/srv/www/n3o.thegraveyard.org/windows/
DONE
fi

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
