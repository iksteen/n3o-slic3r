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

This is intended for local packaging. Before publishing to AUR or
using release tarballs, replace the worktree snapshot in `prepare()`
with a normal `source=...` tarball pinning a release version (and
either bundle the OrcaSlicer submodule revision into the tarball or
pin it as a second source entry).
