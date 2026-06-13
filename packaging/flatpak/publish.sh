#!/usr/bin/env bash
# Release publish for the n3o-slic3r flatpak (PR-9-3): build, GPG-sign,
# export an ostree repo, and generate the .flatpakref for a self-hosted,
# signed distribution channel. (build.sh is the unsigned dev-iteration
# path; this is the signed release path.)
#
# Config (env):
#   N3O_BASE_URL        Public HTTPS base URL of the site; the flatpak repo is
#                       served from <base>/repo (baked into the .flatpakref).
#                       Default: https://n3o.thegraveyard.org
#   N3O_FLATPAK_GPG_KEY Signing key fingerprint. Defaults to the
#                       dedicated project key created for PR-9-3.
#   N3O_PUBLISH_DEST    Optional rsync/ssh destination *base*, e.g.
#                       user@host:/srv/www/n3o.thegraveyard.org. This channel
#                       uploads the repo + ref to <dest>/repo; when unset, it
#                       just prints the manual steps.
#
# See packaging/flatpak/PUBLISHING.md for the hosting setup.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
appid=org.thegraveyard.n3o-slic3r

# Single base URL for the whole site; the flatpak repo is served from <base>/repo.
base_url="${N3O_BASE_URL:-https://n3o.thegraveyard.org}"
repo_url="${base_url%/}/repo"
# Dedicated n3o-slic3r release signing key (PR-9-3). Override to sign
# with a different key.
key="${N3O_FLATPAK_GPG_KEY:-B3D305B467D790E9328FFDF3D0B98FE70335DC53}"

gen="${here}/.gen"
builddir="${here}/.build"
pubrepo="${here}/.publish-repo"
mkdir -p "${gen}"

# Resolve the manifest template + regenerate the OrcaSlicer source
# tarball (mirrors build.sh's prep).
sed "s|@REPO@|${repo}|g" "${here}/${appid}.yml" > "${gen}/${appid}.yml"
git -C "${repo}/external/OrcaSlicer" archive --format=tar --prefix=OrcaSlicer/ HEAD \
  -o "${gen}/orca-src.tar"

echo ":: signed build + export into ${pubrepo} (key ${key})"
flatpak-builder \
  --user \
  --force-clean \
  --state-dir="${here}/.flatpak-builder" \
  --gpg-sign="${key}" \
  --repo="${pubrepo}" \
  "${builddir}" \
  "${gen}/${appid}.yml"

echo ":: sign repo metadata + static deltas"
flatpak build-update-repo \
  --generate-static-deltas \
  --prune \
  --gpg-sign="${key}" \
  "${pubrepo}"

echo ":: export the signing public key for the ref"
gpg --export "${key}" > "${gen}/${appid}.gpg"

echo ":: generate ${appid}.flatpakref"
cat > "${gen}/${appid}.flatpakref" <<REF
[Flatpak Ref]
Name=${appid}
Branch=master
Url=${repo_url}
GPGKey=$(base64 -w0 "${gen}/${appid}.gpg")
IsRuntime=false
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
Title=n3o-slic3r
REF

echo
echo "Done."
echo "  signed repo:  ${pubrepo}"
echo "  flatpakref:   ${gen}/${appid}.flatpakref"

if [[ -n "${N3O_PUBLISH_DEST:-}" ]]; then
  dest="${N3O_PUBLISH_DEST%/}/repo"
  echo
  echo ":: uploading to ${dest}/ (N3O_PUBLISH_DEST set)"
  # Repo contents first — `--delete` prunes stale ostree objects (and the
  # ref, re-added next) so the served tree matches the freshly-pruned repo.
  rsync -a --delete "${pubrepo}/" "${dest}/"
  # Then the ref alongside it, so it resolves at
  # ${repo_url}/${appid}.flatpakref.
  rsync -a "${gen}/${appid}.flatpakref" "${dest}/"
  echo ":: uploaded."
  echo
  echo "Install on a clean machine (signed; no --no-gpg-verify needed):"
  echo "  flatpak install --from ${repo_url}/${appid}.flatpakref"
else
  cat <<DONE

Set N3O_PUBLISH_DEST=<rsync/ssh dest base> (e.g. user@host:/srv/www/n3o.thegraveyard.org)
to upload automatically (this channel uploads to <dest>/repo), or publish by hand:
  1. Upload the repo so it's served at ${repo_url}:
       rsync -a --delete "${pubrepo}/" your-server:/srv/www/n3o.thegraveyard.org/repo/
  2. Upload the ref alongside it:
       scp "${gen}/${appid}.flatpakref" your-server:/srv/www/n3o.thegraveyard.org/repo/
       (so it resolves at ${repo_url}/${appid}.flatpakref)

Install on a clean machine (signed; no --no-gpg-verify needed):
  flatpak install --from ${repo_url}/${appid}.flatpakref
DONE
fi
