# n3o-slic3r

A modern desktop slicer UI built on Tauri 2 + React + TypeScript, driving
OrcaSlicer's `libslic3r` engine through the
[`orca-slicer-ffi`](https://github.com/iksteen/orca-slicer-ffi) C/Rust shim.

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
│  - depends on slic3r-ffi (path)                         │
└──────────────────────┬──────────────────────────────────┘
                       │  Rust → C FFI
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r_ffi.so  (orca-slicer-ffi submodule)          │
│  - flat C API over libslic3r                            │
└──────────────────────┬──────────────────────────────────┘
                       │  static link
┌──────────────────────▼──────────────────────────────────┐
│  libslic3r (OrcaSlicer submodule, pinned to a commit)   │
└─────────────────────────────────────────────────────────┘
```

## Build

Tested on Linux (Arch). macOS should work with minor CMake adjustments;
Windows needs symbol-visibility annotations on the C API.

### 1. System prerequisites

- `cmake` (4.x ok), `ninja`
- A C/C++ toolchain (gcc or clang)
- OrcaSlicer's Linux deps: see `external/orca-slicer-ffi/external/OrcaSlicer/scripts/linux.d/` for the per-distro list (`gtk3`, `dbus`, `webkit2gtk`, `mesa`, …)
- Rust toolchain (stable) and Node 20+

On Arch, the relevant packages are:
```bash
sudo pacman -S cmake ninja gcc gtk3 webkit2gtk dbus mesa wayland-protocols \
               extra-cmake-modules curl pkgconf
```

### 2. Pull the submodules

```bash
git submodule update --init --recursive
```

This brings in `external/orca-slicer-ffi` and, nested under it,
`external/orca-slicer-ffi/external/OrcaSlicer` (pinned to a specific upstream
commit).

### 3. Build orca-slicer-ffi

The slicer engine has a heavy dependency tree (Boost, CGAL, OCCT, TBB,
OpenVDB, etc.) that must be built first.

```bash
cd external/orca-slicer-ffi

# One-time, ~30 minutes: builds OrcaSlicer's deps tree under
# external/OrcaSlicer/deps/build/. Skip if you've already done this.
./scripts/build.sh deps

# Build libslic3r_ffi.so (~15 min cold, fast incremental):
./scripts/build.sh build

cd ../..
```

Verify the output exists:
```
external/orca-slicer-ffi/build/ffi/RelWithDebInfo/libslic3r_ffi.so
```

The Rust crate's `build.rs` (in `external/orca-slicer-ffi/bindings/rust/`)
locates this automatically when `src-tauri` builds.

### 4. Install JS deps + run the dev server

```bash
npm install
npm run tauri dev
```

First build of `src-tauri` is slow (~5 min) because Cargo compiles the
Tauri stack and the `slic3r-ffi` crate runs `bindgen` over the C header.
Incremental rebuilds are fast.

### 5. Smoke test

In the running app:
1. The header should show `OrcaSlicer libslic3r_ffi v0 · 737 options registered`.
2. Type `perimeter` in the search box → table of perimeter-related options.
3. Paste a path to an STL file into the slicer panel (e.g. `external/orca-slicer-ffi/external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`), click Slice. You should see `wrote /tmp/n3o-out.gcode`.

## Production build

```bash
npm run tauri build
```

Note: the produced binary dynamically links against `libslic3r_ffi.so.0` via
rpath. For distribution you'll want to either bundle the `.so` next to the
binary or install it to a standard system location; the current `build.rs`
uses a dev-convenience rpath pointing into the build tree.

## Upgrading orca-slicer-ffi

```bash
cd external/orca-slicer-ffi
git fetch
git checkout <tag-or-commit>
git submodule update --init --recursive   # if OrcaSlicer pin moved too
cd ../..
./external/orca-slicer-ffi/scripts/build.sh build
git add external/orca-slicer-ffi
git commit -m "Bump orca-slicer-ffi to <ref>"
```

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
