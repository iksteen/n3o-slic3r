#!/usr/bin/env bash
# Host-side flatpak build wrapper (PR-9-2).
#
# Resolves the @REPO@ placeholder in the manifest template to this
# checkout's absolute path, then runs flatpak-builder. Sources come from
# the local committed git state (see the manifest), so commit your work
# before building — uncommitted changes outside packaging/flatpak/ won't
# be picked up.
#
# Prereq: the build toolchain comes from three SDK extensions — node22,
# rust-stable, llvm21 — at the org.gnome.Sdk//50 freedesktop base (25.08).
# flatpak-builder won't install them and won't hard-fail when they're
# absent, so this script preflight-checks them. See PUBLISHING.md.
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
# Output ostree repo (publish.sh points this at .publish-repo). Optional GPG
# signing when N3O_GPG_KEY is set (publish.sh sets it; the dev build is unsigned).
repodir="${FLATPAK_REPO:-${here}/.repo}"
sign=()
[[ -n "${N3O_GPG_KEY:-}" ]] && sign=(--gpg-sign="${N3O_GPG_KEY}")
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

# Fail fast on the host if the build toolchain extensions aren't installed,
# rather than ~minutes into flatpak-builder with "npm: command not found".
fdbranch=25.08 # freedesktop base of org.gnome.Sdk//50 (bump with runtime-version)
missing=()
for ext in node22 rust-stable llvm21; do
  ref="org.freedesktop.Sdk.Extension.${ext}//${fdbranch}"
  flatpak info "${ref}" >/dev/null 2>&1 || missing+=("${ref}")
done
if (( ${#missing[@]} )); then
  {
    echo "error: missing flatpak SDK extension(s) the build needs:"
    printf '  - %s\n' "${missing[@]}"
    echo "install with:"
    echo "  flatpak install flathub ${missing[*]}"
  } >&2
  exit 1
fi

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
  "${sign[@]}" \
  "${extra[@]}" \
  "${builddir}" \
  "${gen}/${appid}.yml"

# Regenerate the repo summary + appstream branch. flatpak-builder --repo writes
# the app commit but not the appstream/appstream2 refs, so without this a
# `flatpak update` against a local remote pointing here fails with
# "No such ref 'appstream2/<arch>'". publish.sh runs this too, with static
# deltas + prune on top.
flatpak build-update-repo "${sign[@]}" "${repodir}"

if [[ "${1:-}" == "--run" ]]; then
  flatpak-builder --run "${builddir}" "${gen}/${appid}.yml" "${appid}" || \
    flatpak run "${appid}"
fi
