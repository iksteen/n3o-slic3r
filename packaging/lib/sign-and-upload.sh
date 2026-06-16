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
#   n3o_sign "$art" <label>        # GPG-sign (if a key is set) + print; sets $sig
#   n3o_upload "$art" "$sig"       # upload (or print manual steps)
#
# Each channel's build.sh signs (so a `build` produces the final, signed
# artifact) and its publish.sh uploads. The caller owns artifact resolution and
# any per-OS INSTALL instructions.
#
# Signing is OPTIONAL: with N3O_GPG_KEY unset, n3o_sign is a no-op (no default
# key) and n3o_upload ships the bare artifact.

# Resolve the release-key + site-URL config. Requires `repo`.
# Sets: key (may be empty → unsigned), base_url, url, keyfile, keyname.
n3o_signing_init() {
  # Release key fingerprint. Unset → unsigned build (no default: we never
  # sign with a key the operator didn't ask for).
  key="${N3O_GPG_KEY:-}"
  # Single base URL for the whole site; this channel serves from <base>/pkg.
  base_url="${N3O_BASE_URL:-https://n3o.thegraveyard.org}"
  url="${base_url%/}/pkg"
  keyfile="${repo}/packaging/flatpak/n3o-slic3r-signing-key.asc"
  keyname="$(basename "${keyfile}")"
}

# GPG-sign $1 (detached) and print the result — unless no key is set, in which
# case it's a no-op and `sig` is left empty. $2 is a noun for the output line
# (default "artifact"). Sets `sig` for the caller. Requires n3o_signing_init.
n3o_sign() {
  local art="$1" label="${2:-artifact}"

  if [[ -z "${key}" ]]; then
    sig=""
    echo
    echo "Built (unsigned — set N3O_GPG_KEY to sign):"
    printf '  %-11s%s\n' "${label}:" "${art}"
    return
  fi

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

# Upload artifact $1 (+ its signature $2 if non-empty + the public key) to
# $N3O_PUBLISH_DEST/pkg when set, else print the manual steps. An empty $2 means
# the build was unsigned — only the artifact ships. Requires n3o_signing_init.
n3o_upload() {
  local art="$1" sig="${2:-}"
  # Files to ship: artifact always; signature + public key only when the
  # signature actually exists (callers pass the expected .sig path regardless,
  # so gate on the file, not the string).
  local files=("${art}")
  if [[ -n "${sig}" && -f "${sig}" ]]; then
    files+=("${sig}")
    [[ -f "${keyfile}" ]] && files+=("${keyfile}")
  fi

  if [[ -n "${N3O_PUBLISH_DEST:-}" ]]; then
    local dest="${N3O_PUBLISH_DEST%/}/pkg"
    echo
    echo ":: uploading to ${dest}/ (N3O_PUBLISH_DEST set)"
    rsync -a "${files[@]}" "${dest}/"
    echo ":: uploaded."
  else
    cat <<DONE

Set N3O_PUBLISH_DEST=<rsync/ssh dest base> (e.g.
user@host:/srv/www/n3o.thegraveyard.org) to upload automatically (this channel
uploads to <dest>/pkg), or by hand:
  rsync -a ${files[*]} your-server:/srv/www/n3o.thegraveyard.org/pkg/
DONE
  fi
}
