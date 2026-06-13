# Publishing the n3o-slic3r flatpak (self-hosted, signed)

The MVP distribution channel is a **self-hosted, GPG-signed flatpak
ostree repo** served over HTTPS, installed via a `.flatpakref`. (Flathub
submission is post-MVP — see `docs/dev/tickets/phase-9.md`.)

## Prerequisites

The app builds in-sandbox and pulls its toolchain from three SDK
extensions at the `org.gnome.Sdk//50` freedesktop base (**25.08**). Install
them before `build.sh` / `publish.sh` (both preflight-check and will name
any that are missing):

```sh
flatpak install flathub \
  org.freedesktop.Sdk.Extension.node22//25.08 \
  org.freedesktop.Sdk.Extension.rust-stable//25.08 \
  org.freedesktop.Sdk.Extension.llvm21//25.08
```

flatpak-builder does not auto-install these and does not hard-fail when
they're absent — a missing one would otherwise surface only as a cryptic
`command not found` mid-build (`npm`, then `cargo`, then `clang`). Bump the
`25.08` branch if the manifest's GNOME `runtime-version` changes; it tracks
the SDK's freedesktop base.

## Signing key

Releases are signed with a **dedicated project key** (separate from any
personal key):

- Fingerprint: `B3D305B467D790E9328FFDF3D0B98FE70335DC53`
- UID: `n3o-slic3r release signing key`
- Public key (commit-tracked, for out-of-band verification):
  [`n3o-slic3r-signing-key.asc`](n3o-slic3r-signing-key.asc)

The secret key lives in the maintainer's GnuPG keyring. It is
passphraseless so `publish.sh` can sign non-interactively; if you'd
rather protect it, add a passphrase (`gpg --change-passphrase`) and run
publish behind an unlocked `gpg-agent`. To rotate: generate a new key,
update `N3O_FLATPAK_GPG_KEY` (or the default in `publish.sh`), re-export
`n3o-slic3r-signing-key.asc`, and re-publish — clients pick up the new
key from the updated `.flatpakref`.

## One-time hosting setup

The repo is served from **`https://n3o.thegraveyard.org/repo/`**. The
marketing site (`docs/site/`) lives at the domain root and the ostree
repo sits under `/repo/`, so a publish's `--delete` only ever prunes the
repo and never touches the site. The ostree repo is just static files —
any web server (nginx, Caddy, object storage + CDN) works. No
server-side software is needed.

## Per-release publish

From the repo root (the served base URL defaults to
`https://n3o.thegraveyard.org`; the flatpak repo is served from `<base>/repo`,
override with `N3O_BASE_URL`):

```bash
packaging/flatpak/publish.sh
```

`publish.sh` (the signed release path; `build.sh` is the unsigned dev
path):

1. resolves the manifest + regenerates the OrcaSlicer source tarball,
2. runs a **GPG-signed** `flatpak-builder` export into
   `packaging/flatpak/.publish-repo/` (modules are cached, so this is
   fast after the first full build),
3. signs the repo summary + static deltas
   (`flatpak build-update-repo --gpg-sign`),
4. writes `packaging/flatpak/.gen/org.thegraveyard.n3o-slic3r.flatpakref`
   with the repo URL and the embedded public key.

Then upload both to your host so they resolve under `<N3O_BASE_URL>/repo`.
Set **`N3O_PUBLISH_DEST`** to the site *base* rsync/ssh destination and
`publish.sh` does the upload for you to `<dest>/repo` (repo with `--delete`,
then the ref alongside it):

```bash
N3O_PUBLISH_DEST="your-server:/srv/www/n3o.thegraveyard.org" \
  packaging/flatpak/publish.sh
```

Or upload by hand (what the script prints when `N3O_PUBLISH_DEST` is
unset):

```bash
rsync -a --delete packaging/flatpak/.publish-repo/  your-server:/srv/www/n3o.thegraveyard.org/repo/
scp packaging/flatpak/.gen/org.thegraveyard.n3o-slic3r.flatpakref \
    your-server:/srv/www/n3o.thegraveyard.org/repo/
```

## Install (end user, clean machine)

```bash
flatpak install --from https://n3o.thegraveyard.org/repo/org.thegraveyard.n3o-slic3r.flatpakref
```

The ref carries the signing key, so the install verifies the signature
— no `--no-gpg-verify`. The ref's `RuntimeRepo` points at Flathub so the
GNOME 50 runtime is pulled automatically if the user doesn't have it.

## Licensing

n3o-slic3r is AGPL-3.0-or-later (the libslic3r linkage forces it). The
corresponding source is the project repository linked from the app's
AppStream metadata (`<url type="homepage">`). Keep that URL pointing at
publicly available source for every published build.
