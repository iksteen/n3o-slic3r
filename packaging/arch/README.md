# Arch Linux Package

Build the local n3o-slic3r package with:

```sh
cd packaging/arch
makepkg -s
```

This produces a single package: `n3o-slic3r-<ver>-<rel>-x86_64.pkg.tar.zst`.
Install with:

```sh
sudo pacman -U ./n3o-slic3r-0.1.0-1-x86_64.pkg.tar.zst
```

The PKGBUILD snapshots the worktree via `git ls-files --cached
--recurse-submodules`, which pulls in tracked files from both the
top-level repo and the OrcaSlicer submodule. **Commit-first
discipline**: uncommitted edits (top-level or submodule) are *not*
packaged — commit your changes before running `makepkg` if you
want them included.

**No caching, no shared artifacts**: each `makepkg` run starts
from a fresh snapshot, then builds OrcaSlicer's deps tree
(Boost/CGAL/OCCT/TBB/OpenVDB/...), the FFI shim, libslic3r, the
Vite frontend, and the Tauri binary all from source. The full run
takes ~35-40 min on a fast machine (~17 min for the deps tree,
~15 min for libslic3r + the FFI shim, the rest for the frontend
+ Tauri bundling).

The resulting package installs:

- `/usr/bin/n3o-slic3r` — the Tauri-bundled desktop binary
- `/usr/lib/libslic3r_ffi.so{,.0}` — the FFI shim libslic3r is wrapped through
- `/usr/lib/<bundle-id>/resources/profiles/` — bundled vendor profiles
- `/usr/share/applications/n3o-slic3r.desktop` — Tauri-generated desktop entry
- `/usr/share/icons/hicolor/.../n3o-slic3r.png` — Tauri-bundled icons
- `/usr/share/licenses/n3o-slic3r/LICENSE` — AGPL-3.0-or-later

## Publishing (signed, for `pacman -U`)

`publish.sh` builds the package, GPG-signs it with the dedicated project
key (the same one the flatpak channel uses), and uploads the signed
`.pkg.tar.zst` so users can install it with a bare `pacman -U <url>` —
not a full pacman repo, just one signed file served over HTTPS. It
follows the same commit-first discipline as a plain `makepkg` (it builds
from the committed worktree snapshot).

```sh
N3O_PUBLISH_DEST="user@host:/srv/www/n3o.thegraveyard.org" \
  packaging/arch/publish.sh
```

`N3O_PUBLISH_DEST` is the site *base*; this channel uploads to `<dest>/pkg`.
With it unset, the script builds + signs and prints the manual upload + install
steps instead. Override the signing key with `N3O_ARCH_GPG_KEY` and the served
base URL with `N3O_BASE_URL` (default `https://n3o.thegraveyard.org`; this
channel serves from `<base>/pkg`). The public key is committed at
`packaging/flatpak/n3o-slic3r-signing-key.asc` and uploaded alongside the
package.

End users install with a one-time key import, then `pacman -U`:

```sh
curl -fsSLO https://n3o.thegraveyard.org/pkg/n3o-slic3r-signing-key.asc
sudo pacman-key --add n3o-slic3r-signing-key.asc
sudo pacman-key --lsign-key B3D305B467D790E9328FFDF3D0B98FE70335DC53
sudo pacman -U https://n3o.thegraveyard.org/pkg/n3o-slic3r-<ver>-1-x86_64.pkg.tar.zst
```

pacman fetches the `.sig` alongside the package and verifies it against
the now-trusted key (Arch's default `SigLevel` requires package
signatures).

## AUR / release tarballs

This is intended for local + self-hosted packaging. Before publishing to
AUR or using release tarballs, replace the worktree snapshot in `prepare()`
with a normal `source=...` tarball pinning a release version (and
either bundle the OrcaSlicer submodule revision into the tarball or
pin it as a second source entry).
