#!/usr/bin/env bash
# Host-side flatpak build wrapper (PR-9-2).
#
# Resolves the @REPO@ placeholder in the manifest template to this
# checkout's absolute path, then runs flatpak-builder. Sources come from
# the local committed git state (see the manifest), so commit your work
# before building — uncommitted changes outside packaging/flatpak/ won't
# be picked up.
#
# Usage:
#   packaging/flatpak/build.sh            # build (+ export to local repo)
#   packaging/flatpak/build.sh --run      # build, then flatpak-run it
#   packaging/flatpak/build.sh --install  # build, then install --user
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
appid=org.thegraveyard.n3o-slic3r

gen="${here}/.gen"
builddir="${here}/.build"
repodir="${here}/.repo"
mkdir -p "${gen}"

# Resolve the manifest template.
sed "s|@REPO@|${repo}|g" "${here}/${appid}.yml" > "${gen}/${appid}.yml"

# OrcaSlicer source as a clean tarball of the pinned submodule commit
# (tracked files only). Sidesteps flatpak-builder's git-lfs-over-full-
# history fetch; see the manifest comment.
git -C "${repo}/external/OrcaSlicer" archive --format=tar --prefix=OrcaSlicer/ HEAD \
  -o "${gen}/orca-src.tar"

extra=()
case "${1:-}" in
  --run) ;;
  --install) extra+=(--install) ;;
  "" ) ;;
  *) echo "unknown arg: $1" >&2; exit 2 ;;
esac

# --user installs the runtime/sdk from the user flathub remote;
# --force-clean wipes the previous build tree; sources are cached under
# .flatpak-builder/ between runs.
# --state-dir keeps flatpak-builder's (large) download/build cache under
# packaging/flatpak/ regardless of the invoking CWD — otherwise it lands
# in the CWD (e.g. a 29 GB .flatpak-builder/ at the repo root).
flatpak-builder \
  --user \
  --force-clean \
  --state-dir="${here}/.flatpak-builder" \
  --repo="${repodir}" \
  "${extra[@]}" \
  "${builddir}" \
  "${gen}/${appid}.yml"

if [[ "${1:-}" == "--run" ]]; then
  flatpak-builder --run "${builddir}" "${gen}/${appid}.yml" "${appid}" || \
    flatpak run "${appid}"
fi
