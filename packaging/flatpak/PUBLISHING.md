# Publishing the n3o-slic3r flatpak (self-hosted, signed)

The MVP distribution channel is a **self-hosted, GPG-signed flatpak
ostree repo** served over HTTPS, installed via a `.flatpakref`. (Flathub
submission is post-MVP — see `docs/tickets/phase-9.md`.)

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

Pick an HTTPS location to serve a static directory, e.g.
`https://dl.example.org/n3o-slic3r/`. The ostree repo is just static
files — any web server (nginx, Caddy, object storage + CDN) works. No
server-side software is needed.

## Per-release publish

From the repo root, with the public URL set:

```bash
N3O_FLATPAK_REPO_URL="https://dl.example.org/n3o-slic3r" \
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

Then upload both to your host so they resolve under `$N3O_FLATPAK_REPO_URL`:

```bash
rsync -a --delete packaging/flatpak/.publish-repo/  your-server:/srv/www/n3o-slic3r/
scp packaging/flatpak/.gen/org.thegraveyard.n3o-slic3r.flatpakref \
    your-server:/srv/www/n3o-slic3r/
```

## Install (end user, clean machine)

```bash
flatpak install --from https://dl.example.org/n3o-slic3r/org.thegraveyard.n3o-slic3r.flatpakref
```

The ref carries the signing key, so the install verifies the signature
— no `--no-gpg-verify`. The ref's `RuntimeRepo` points at Flathub so the
GNOME 50 runtime is pulled automatically if the user doesn't have it.

## Licensing

n3o-slic3r is AGPL-3.0-or-later (the libslic3r linkage forces it). The
corresponding source is the project repository linked from the app's
AppStream metadata (`<url type="homepage">`). Keep that URL pointing at
publicly available source for every published build.
