#!/usr/bin/env bash
# Shared signing + upload helpers for the single-file publish channels
# (arch, windows-cross, macos-cross — all serve one signed artifact from
# <base>/pkg). The flatpak channel is an ostree repo and does its own
# thing, so it does NOT use this.
#
# Source it from a build.sh / publish.sh that has already set `repo`, then:
#
#   n3o_signing_init               # sets: key, base_url, url, keyfile, keyname
#   <resolve the built artifact into $art>
#   n3o_sign "$art" <label>        # GPG-sign + print; sets $sig
#   n3o_upload "$art" "$sig"       # upload (or print manual steps)
#
# `n3o_sign_and_upload` is the two combined, for channels (arch, windows) that
# sign + upload in one publish step. The macOS channel splits them: build.sh
# signs (so a `build` produces the final, signed artifact), publish.sh uploads.
# The caller owns artifact resolution and any per-OS INSTALL instructions.

# Resolve the shared release-key + site-URL config. Requires `repo`.
# Sets: key, base_url, url, keyfile, keyname.
n3o_signing_init() {
  # Shared dedicated release key (same across all channels). Override to
  # sign with a different key.
  key="${N3O_GPG_KEY:-B3D305B467D790E9328FFDF3D0B98FE70335DC53}"
  # Single base URL for the whole site; this channel serves from <base>/pkg.
  base_url="${N3O_BASE_URL:-https://n3o.thegraveyard.org}"
  url="${base_url%/}/pkg"
  keyfile="${repo}/packaging/flatpak/n3o-slic3r-signing-key.asc"
  keyname="$(basename "${keyfile}")"
}

# GPG-sign $1 (detached) and print the result. $2 is a noun for the "Built +
# signed:" line (default "artifact"). Sets `sig` for the caller. Requires
# n3o_signing_init to have run.
n3o_sign() {
  local art="$1" label="${2:-artifact}"

  echo ":: GPG sign $(basename "${art}") (key ${key})"
  # Detached signature next to the artifact; users verify with `gpg --verify`
  # (pacman -U fetches `<url>.sig` automatically on the arch channel).
  gpg --batch --yes --local-user "${key}" --detach-sign "${art}"
  sig="${art}.sig"
  [[ -f "${sig}" ]] || { echo "error: signing did not produce ${sig}" >&2; exit 1; }

  echo
  echo "Built + signed:"
  printf '  %-11s%s\n' "${label}:" "${art}"
  printf '  %-11s%s\n' "signature:" "${sig}"
}

# Upload artifact $1 + its signature $2 + the public key to $N3O_PUBLISH_DEST/pkg
# when set, else print the manual steps. Requires n3o_signing_init to have run.
n3o_upload() {
  local art="$1" sig="$2"
  if [[ -n "${N3O_PUBLISH_DEST:-}" ]]; then
    local dest="${N3O_PUBLISH_DEST%/}/pkg"
    echo
    echo ":: uploading to ${dest}/ (N3O_PUBLISH_DEST set)"
    # The artifact + its detached signature + the public key so users can
    # import, trust, and verify it.
    rsync -a "${art}" "${sig}" "${dest}/"
    [[ -f "${keyfile}" ]] && rsync -a "${keyfile}" "${dest}/"
    echo ":: uploaded."
  else
    cat <<DONE

Set N3O_PUBLISH_DEST=<rsync/ssh dest base> (e.g.
user@host:/srv/www/n3o.thegraveyard.org) to upload automatically (this channel
uploads to <dest>/pkg), or by hand:
  rsync -a "${art}" "${sig}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
  rsync -a "${keyfile}" your-server:/srv/www/n3o.thegraveyard.org/pkg/
DONE
  fi
}

# Sign + upload in one step — for channels that do both in publish.sh (arch,
# windows). Sets `sig`.
n3o_sign_and_upload() {
  n3o_sign "$1" "${2:-artifact}"
  n3o_upload "$1" "${sig}"
}
