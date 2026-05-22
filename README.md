# n3o-slic3r

A modern desktop slicer UI built on Tauri 2 + React + TypeScript, driving
OrcaSlicer's `libslic3r` engine through a vendored Rust+C FFI shim
(`crates/slic3r-ffi/`).

The goal is to use libslic3r as a slicing engine (well-tested, ~600 settings,
calibration features) without inheriting OrcaSlicer's preset/profile system,
which has accreted from the Slic3r → PrusaSlicer → Bambu Studio lineage.

## Status

Early prototype. The Tauri renderer can call into the Rust backend, which
calls into the C shim, which drives libslic3r:

- `slicer_info` — version banner + option count (737 options registered)
- `slicer_options(filter)` — introspect ConfigOptionDefs (key, type, label,
  category, default)
- `slicer_slice(model_path, out_path)` — load STL/3MF/OBJ/STEP and emit G-code

Bambu A1 mini 3MFs slice end to end and produce gcode safe to send to the
printer (real Bambu start/end sequences — `M1002`, `G29 A1`, `M620` AMS
load/unload, `M983`/`M984` extrusion calibration). Other printer profiles
should work similarly; only A1 mini has been smoke-tested.

No 3D viewer, no preset model, no calibration tools yet — that's the work.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  React / TypeScript renderer  (src/)                    │
└──────────────────────┬──────────────────────────────────┘
                       │  Tauri IPC (JSON over local channel)
┌──────────────────────▼──────────────────────────────────┐
│  Rust backend  (src-tauri/)                             │
│  - Tauri commands  (slicer_info, slicer_options, …)     │
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

The FFI is vendored as a workspace member (`crates/slic3r-ffi/`) rather
than an external crate. We're patching it heavily and opinionatedly while
figuring out what shape it wants to be; an external crate would be
premature. Once the API stabilizes, the crate can move back to its own
repo unchanged.

## Build

Tested on Linux (Arch). macOS should work with minor CMake adjustments;
Windows would need symbol-visibility annotations on the C API.

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
`build/slic3r-ffi/RelWithDebInfo/libslic3r_ffi.so`.

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

In the running app:
1. The header should show `OrcaSlicer libslic3r_ffi v0 · 737 options registered`.
2. Type `perimeter` in the search box → table of perimeter-related options.
3. Paste a path to a model file into the slicer panel and click Slice. For
   a quick verify use the bundled test STL:
   `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`.

For headless slicing without the UI:
```bash
cargo run -p slic3r-ffi --release --example slice -- <model> /tmp/out.gcode
```

## Production build

```bash
npm run tauri build
```

The produced binary dynamically links against `libslic3r_ffi.so.0` via rpath
to the build tree (dev convenience). For distribution, bundle the `.so` next
to the binary or install it to a system library path; the current `build.rs`
rpath is not portable.

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
  `docs/libslic3r-workarounds.md` before bumping the OrcaSlicer
  submodule — removing one of these without confirming upstream
  fixed the root cause silently reintroduces hard-to-debug failures.

## Upgrading OrcaSlicer

```bash
cd external/OrcaSlicer
git fetch
git checkout <tag-or-commit>
cd ..
./scripts/build.sh build
git add external/OrcaSlicer
git commit -m "Bump OrcaSlicer to <ref>"
```

Bumps that touch the libslic3r tool-ordering or config setup may require
matching changes in `ffi/slic3r_ffi.cpp` — see the comments there for the
specific pre-`apply` normalization the shim does to work around upstream's
GUI-side assumptions.

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
