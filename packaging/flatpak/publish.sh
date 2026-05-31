#!/usr/bin/env bash
# Release publish for the n3o-slic3r flatpak (PR-9-3): build, GPG-sign,
# export an ostree repo, and generate the .flatpakref for a self-hosted,
# signed distribution channel. (build.sh is the unsigned dev-iteration
# path; this is the signed release path.)
#
# Config (env):
#   N3O_FLATPAK_REPO_URL   REQUIRED. Public HTTPS base URL the repo is
#                          served from, e.g. https://dl.example.org/n3o
#   N3O_FLATPAK_GPG_KEY    Signing key fingerprint. Defaults to the
#                          dedicated project key created for PR-9-3.
#
# After it runs, upload the repo + ref to your server (see the printed
# instructions and packaging/flatpak/PUBLISHING.md).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
appid=org.thegraveyard.n3o-slic3r

: "${N3O_FLATPAK_REPO_URL:?set N3O_FLATPAK_REPO_URL to the public HTTPS base URL the repo is served from}"
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
Url=${N3O_FLATPAK_REPO_URL}
GPGKey=$(base64 -w0 "${gen}/${appid}.gpg")
IsRuntime=false
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
Title=n3o-slic3r
REF

cat <<DONE

Done.
  signed repo:  ${pubrepo}
  flatpakref:   ${gen}/${appid}.flatpakref

Publish (one-time + per-release):
  1. Upload the repo so it's served at \$N3O_FLATPAK_REPO_URL:
       rsync -a --delete "${pubrepo}/" your-server:/srv/www/n3o/
  2. Upload the ref alongside it:
       scp "${gen}/${appid}.flatpakref" your-server:/srv/www/n3o/
       (so it resolves at \$N3O_FLATPAK_REPO_URL/${appid}.flatpakref)

Install on a clean machine (signed; no --no-gpg-verify needed):
  flatpak install --from ${N3O_FLATPAK_REPO_URL}/${appid}.flatpakref
DONE
