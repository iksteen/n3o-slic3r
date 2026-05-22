# n3o-slic3r

A modern desktop slicer UI built on Tauri 2 + React + TypeScript, driving
OrcaSlicer's `libslic3r` engine through a vendored C/Rust FFI shim
(`ffi/` + `bindings/rust/`).

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
                       │  Rust path dep
┌──────────────────────▼──────────────────────────────────┐
│  slic3r-ffi  (bindings/rust/)                           │
│  - bindgen + safe wrapper over slic3r_ffi.h             │
└──────────────────────┬──────────────────────────────────┘
                       │  Rust → C FFI
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r_ffi.so  (ffi/, built via cmake)              │
│  - flat C API over libslic3r                            │
└──────────────────────┬──────────────────────────────────┘
                       │  static link
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r (external/OrcaSlicer submodule, pinned)      │
└─────────────────────────────────────────────────────────┘
```

The FFI layer (`ffi/`, `bindings/rust/`) is vendored into this repo rather
than referenced as an external crate. We're patching it heavily and
opinionatedly while figuring out what shape it wants to be; an external crate
would be premature. Once the API stabilizes it can move back to its own repo.

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

### 3. Build the FFI .so

OrcaSlicer's dependency tree (Boost, CGAL, OCCT, TBB, OpenVDB, etc.) must
be built first; then libslic3r and the shim.

```bash
# One-time, ~30 minutes: builds OrcaSlicer's deps tree under
# external/OrcaSlicer/deps/build/. Skip if already done.
./scripts/build.sh deps

# Build libslic3r_ffi.so (~15 min cold, fast incremental):
./scripts/build.sh build
```

Output:
```
build/ffi/RelWithDebInfo/libslic3r_ffi.so
```

`bindings/rust/build.rs` and `src-tauri/build.rs` both default to that path
and set the binary's rpath accordingly.

### 4. Install JS deps + run the dev server

```bash
npm install
npm run tauri dev
```

First build of `src-tauri` is slow (~5 min) because Cargo compiles the
Tauri stack and `slic3r-ffi` runs `bindgen` over the C header. Incremental
rebuilds are fast.

### 5. Smoke test

In the running app:
1. The header should show `OrcaSlicer libslic3r_ffi v0 · 737 options registered`.
2. Type `perimeter` in the search box → table of perimeter-related options.
3. Paste a path to a model file into the slicer panel and click Slice. For
   a quick verify use the bundled test STL:
   `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`.

For headless slicing without the UI:
```bash
cd bindings/rust
cargo run --release --example slice -- <model> /tmp/out.gcode
```

## Production build

```bash
npm run tauri build
```

The produced binary dynamically links against `libslic3r_ffi.so.0` via rpath
to the build tree (dev convenience). For distribution, bundle the `.so` next
to the binary or install it to a system library path; the current `build.rs`
rpath is not portable.

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
