#!/usr/bin/env bash
# Publish channel for the browser demo (demo/README.md): build the static
# bundle and upload it to <base>/demo, served at e.g.
# https://n3o.thegraveyard.org/demo/.
#
# Unlike the pkg channels this ships a *website* (a directory of HTML/JS/CSS),
# not a signed downloadable artifact — so no GPG signing, and it uploads to
# <dest>/demo (not <dest>/pkg).
#
#   N3O_BASE_URL       Site base URL (display only). Default: https://n3o.thegraveyard.org
#   N3O_PUBLISH_DEST   rsync/ssh destination base, e.g.
#                      user@host:/srv/www/n3o.thegraveyard.org — uploads to
#                      <dest>/demo. Unset → prints the manual upload command.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${repo}"

echo ":: building the browser demo"
npm run demo:build

base_url="${N3O_BASE_URL:-https://n3o.thegraveyard.org}"
src="demo/dist/app/"

if [[ -n "${N3O_PUBLISH_DEST:-}" ]]; then
  dest="${N3O_PUBLISH_DEST%/}/demo"
  echo
  echo ":: uploading demo to ${dest}/ (N3O_PUBLISH_DEST set)"
  # --delete: the bundle uses content-hashed asset names, so stale files must be
  # pruned or they accumulate. The /demo subdir is owned wholesale by this build.
  rsync -a --delete "${src}" "${dest}/"
  echo ":: uploaded — ${base_url%/}/demo/"
else
  cat <<DONE

Set N3O_PUBLISH_DEST=<rsync/ssh dest base> (e.g.
user@host:/srv/www/n3o.thegraveyard.org) to upload automatically (this channel
uploads to <dest>/demo), or by hand:
  rsync -a --delete ${src} your-server:/srv/www/n3o.thegraveyard.org/demo/
DONE
fi
