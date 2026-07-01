# n3o-slic3r

[![build](https://github.com/iksteen/n3o-slic3r/actions/workflows/build.yml/badge.svg?branch=main)](https://github.com/iksteen/n3o-slic3r/actions/workflows/build.yml)

A modern desktop slicer UI built on Tauri 2 + React + TypeScript, driving
OrcaSlicer's `libslic3r` engine through a vendored Rust+C FFI shim
(`crates/slic3r-ffi/`).

The goal is to use libslic3r as a slicing engine (well-tested, ~600 settings,
calibration features) without inheriting OrcaSlicer's preset/profile system,
which has accreted from the Slic3r → PrusaSlicer → Bambu Studio lineage.

## Status

A complete, standalone multi-printer slicer. It runs the Bambu Lab A1 mini
and Snapmaker U1 simultaneously (plus a bundled full-size A1 profile) with
the full slice-and-send workflow: a Rust `wgpu` edit viewport and G-code
preview, a rule-based settings cascade with per-key "why is X=Y" tracing,
both printer drivers (Bambu MQTT + Snapmaker mTLS/MQTT) with filament sync
and live camera, and a Lua plugin system for G-code post-processing. It
needs no other slicer installed and produces builds for Linux (arch/flatpak),
Windows, and macOS.

The native project format is `.n3o`; OrcaSlicer `.3mf` projects import
(geometry + settings). The A1 mini + U1 are the hardware-validated pair.
Deep context for contributors lives in `AGENTS.md` and `docs/dev/`.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  React / TypeScript renderer  (src/)                    │
└──────────────────────┬──────────────────────────────────┘
                       │  Tauri IPC (JSON over local channel)
┌──────────────────────▼──────────────────────────────────┐
│  Rust backend  (src-tauri/)                             │
│  - Tauri commands (scene, slice, cascade, drivers, …)   │
└──────────────────────┬──────────────────────────────────┘
                       │  workspace dep
┌──────────────────────▼──────────────────────────────────┐
│  slic3r-ffi  (crates/slic3r-ffi/)                       │
│  - safe Rust wrapper + raw bindgen module               │
│  - build.rs invokes cmake to build libslic3r_ffi.so     │
│  - links = "slic3r_ffi"; emits DEP_SLIC3R_FFI_LIB_DIR   │
└──────────────────────┬──────────────────────────────────┘
                       │  C++ shim (ffi/ inside the crate)
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r_ffi.so   (cmake target, dynamic)             │
│  - flat C API over libslic3r                            │
└──────────────────────┬──────────────────────────────────┘
                       │  static link
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r (external/OrcaSlicer submodule, pinned)      │
└─────────────────────────────────────────────────────────┘
```

The FFI is a first-party, in-house crate vendored as a workspace member
(`crates/slic3r-ffi/`). It patches libslic3r's headless-mode quirks heavily
and opinionatedly (see `docs/dev/libslic3r-workarounds.md`), so it lives in
the tree rather than as an external dependency.

## Build

Linux and macOS build natively; Windows and macOS also cross-compile from
Linux. All four are validated — see `AGENTS.md` and `packaging/<target>/`
for the per-platform specifics.

### 1. System prerequisites

- `cmake` (4.x ok), `ninja`
- A C/C++ toolchain (gcc or clang)
- OrcaSlicer's Linux deps: see `external/OrcaSlicer/scripts/linux.d/` for the
  per-distro list (`gtk3`, `dbus`, `webkit2gtk`, `mesa`, …)
- Rust toolchain (stable) and Node 20+

On Arch:
```bash
sudo pacman -S cmake ninja gcc gtk3 webkit2gtk dbus mesa wayland-protocols \
               extra-cmake-modules curl pkgconf
```

### 2. Pull the submodule

```bash
git submodule update --init --recursive
```

This brings in `external/OrcaSlicer` pinned at a specific upstream commit.

### 3. Build OrcaSlicer's dependency tree

OrcaSlicer's deps (Boost, CGAL, OCCT, TBB, OpenVDB, etc.) are built once
into `external/OrcaSlicer/deps/build/`. Takes ~17 min, idempotent.

```bash
./scripts/build.sh deps
```

Everything beyond this point is driven by `cargo build`. The `slic3r-ffi`
crate's `build.rs` invokes cmake to build `libslic3r_ffi.so` (and
libslic3r transitively) on first build; cmake's own caching keeps
incrementals to a few seconds. Output lands at
`build/slic3r-ffi/<config>/libslic3r_ffi.so`. The default config is
RelWithDebInfo, which OrcaSlicer forces to `-O0` (slow slicing) — set
`N3O_SLIC3R_FFI_CMAKE_CONFIG=Release` for an optimized local engine.

### 4. Install JS deps + run the dev server

```bash
npm install
npm run tauri dev
```

First build is slow (~15 min for libslic3r + a few min for Tauri/bindgen).
Subsequent builds are incremental and fast. The Tauri binary's `RUNPATH`
is set automatically via `DEP_SLIC3R_FFI_LIB_DIR` (the slic3r-ffi crate
declares `links = "slic3r_ffi"` and emits `cargo:LIB_DIR=...` for
downstream consumers).

### 5. Smoke test

The app opens into the workspace (or onboarding, if no printers are
configured). Add a model, pick a printer + filament, and slice — the G-code
preview should render the toolpaths.

For a headless engine check without the UI:
```bash
cargo run -p slic3r-ffi --release --example slice -- <model> /tmp/out.gcode
# quick model: external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl
```

## Production build & packaging

`npm run tauri build` produces a native bundle. For distributable, signed
artifacts use the per-target packaging under `packaging/<target>/` (arch,
flatpak, windows-cross, macos-cross) — each exposes `build.sh` / `publish.sh`
/ `clean.sh`, mirrored as npm `build:<t>` / `publish:<t>` / `clean:<t>` (plus
`build:all` / `publish:all`). See `AGENTS.md` and each target's `README.md`.

## Development gotchas

A few traps that aren't obvious from the toolchain alone.

- **cmake stores absolute source paths.** Moving the project directory on
  disk (or renaming an ancestor directory) invalidates the cmake build
  cache — `cargo build` from the new location refuses with "The source
  directory does not match the source directory that has been set up
  before." The previously-built binary at `target/<profile>/n3o-slic3r`
  also has the old absolute rpath baked in and won't find the .so.
  Recovery is `rm -rf build/ target/` followed by `cargo build`. The
  OrcaSlicer deps tree under `external/OrcaSlicer/deps/build/` survives
  the move (its cmake config files mostly use relative `_IMPORT_PREFIX`),
  so a full deps rebuild isn't needed.

- **Cargo's `rustc-link-arg` doesn't propagate through the dep graph.**
  The slic3r-ffi crate emits its own rpath via `rustc-link-arg`, but
  that only applies when slic3r-ffi itself produces a binary (its
  examples and tests). For downstream binaries like the Tauri app,
  rpath has to be set by the binary-producing crate. We use Cargo's
  `links = "slic3r_ffi"` + `cargo:LIB_DIR=...` metadata channel:
  slic3r-ffi's `build.rs` emits the lib directory, Cargo surfaces it
  as `DEP_SLIC3R_FFI_LIB_DIR` in `src-tauri/build.rs`'s env, and
  src-tauri sets its own rpath from there. If you add another binary
  crate to the workspace that depends on slic3r-ffi, repeat that
  pattern in its `build.rs` — the rpath isn't inherited.

- **Vite watches everything by default.** The dev server's chokidar
  watcher recursively walks the project root on startup. With
  `external/OrcaSlicer/` containing ~750 MB and tens of thousands of
  files, an unconfigured watcher hits `inotify` limits and falls back
  to polling — `npm run tauri dev` startup grinds to a minute+.
  `vite.config.ts` excludes `external/`, `crates/`, `build/`,
  `target/`, `docs/`, `scripts/` from the watch tree. Re-scaffolding
  the Tauri side without preserving those exclusions reintroduces
  the symptom.

- **libslic3r workarounds in the FFI shim.** `crates/slic3r-ffi/ffi/
  slic3r_ffi.cpp` applies several pre-`apply` and post-`apply`
  normalizations to compensate for libslic3r's headless-mode quirks
  (temp dir defaulting to filesystem root, missing `LoadStrategy::
  LoadModel`, uninitialized `is_BBL_printer`, filament_map
  normalization, coEnums serialization). Read
  `docs/dev/libslic3r-workarounds.md` before bumping the OrcaSlicer
  submodule — removing one of these without confirming upstream
  fixed the root cause silently reintroduces hard-to-debug failures.

## Upgrading OrcaSlicer

A pin bump is never just the submodule SHA — it also regenerates everything
derived from upstream (the scraped option tables and every bundled printer/
process/filament profile), and upstream option renames surface as test
breakage. `scripts/sync_orcaslicer.sh` captures the whole dance in one place:

```bash
./scripts/sync_orcaslicer.sh <tag-or-commit>   # omit the ref to regen at the current pin
```

It repins the submodule (dropping the regenerable carry patches), re-runs the
scrapers, re-imports the machine/process/filament profiles, forces a real
libslic3r rebuild, and runs the full test suite (`Release` FFI config). It
leaves everything **staged for review** — eyeball the profile + scraper diffs
and commit the submodule bump alongside them. Bumps that touch libslic3r's
tool-ordering or config setup may also need matching pre-`apply` normalization
in `ffi/slic3r_ffi.cpp`; read `docs/dev/libslic3r-workarounds.md` first.

## License

**AGPL-3.0-or-later**, same as upstream OrcaSlicer.

This is not a choice. `libslic3r_ffi.so` statically absorbs libslic3r, which
is AGPL-3.0. Anything that links against `libslic3r_ffi.so` is a derivative
work and must be AGPL-compatible. n3o-slic3r links against it (through the
`slic3r-ffi` Rust crate), so AGPL applies to this codebase too.

If you wanted a permissive-licensed UI, you would need to either:
- Spawn OrcaSlicer as a subprocess (CLI invocation, "mere aggregation"
  argument applies — but the AGPL network-use clause complicates this for
  web services), or
- Replace libslic3r with a permissively-licensed slicer engine (none exist
  with comparable capability at time of writing).
