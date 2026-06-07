# Windows cross-compilation (Linux → x86_64-pc-windows-msvc)

Goal: build n3o-slic3r for Windows **entirely on Linux** — no Windows host, no
wine — producing real **MSVC-ABI** binaries (so they link against the
`cl.exe`-built world and use WebView2 the way Tauri expects).

Toolchain: **clang-cl + LLD** in MSVC mode, against the MSVC CRT + Windows SDK
fetched by **[cargo-xwin](https://github.com/rust-cross/cargo-xwin)** (which
wraps [`xwin`](https://github.com/Jake-Shadle/xwin)). This is the same ABI a
native MSVC build produces.

## Status (2026-06-07) — feasibility spike

Cross-built **clean** with this toolchain (each a full Ninja build → `.lib`):

| dep | result |
|-----|--------|
| **OCCT** 7.6.0 | C++ compiles clean (the scariest dep) ✅ |
| **TBB** 2021.5.0 | full clean ✅ |
| **zlib** 1.3.1 | clean ✅ |
| **OpenEXR/IlmBase** 2.5.5 (Half) | clean ✅ |

**WIP:** Boost (needs a `b2` clang-win cross config), OpenVDB (needs Boost's
`iostreams`+`system`; it configures + finds all other deps and carries a clang
patch), then the remaining OrcaSlicer deps (CGAL, Cereal, Eigen, NLopt, Qhull,
…) and finally `libslic3r` + the FFI shim + the Tauri app.

**Conclusion:** full Linux→Windows MSVC cross is **viable** — heavy templated
C++ (OCCT, TBB, IlmBase) all cross. The remaining work is build *plumbing*, not
compiler walls.

### Why not OrcaSlicer's own Windows deps build

`external/OrcaSlicer/deps/deps-windows.cmake` is a **native-Windows** superbuild
— Visual Studio generator + `msbuild … INSTALL.vcxproj` per dep. CMake's VS
generators only exist on a Windows-host CMake, so it does not cross. This
directory rebuilds each dep through **Ninja + clang-cl** instead, reusing
OrcaSlicer's version pins + patches (`deps/<Name>/`).

## Two toolchain gaps we had to fix

cargo-xwin's generated toolchain targets *cargo*, not a CMake deps superbuild,
so two things were missing (both baked into the files here):

1. **RC includes** (`rc-sdk-includes.cmake`) — CMake preprocesses `.rc` files
   with `clang-cl -E`, which pulls includes from the *target* dirs, not the C/C++
   `/imsvc` flags. Version-info `.rc` files therefore failed with
   `'windows.h' file not found`. Fix: add the SDK include dirs via
   `include_directories()` from `CMAKE_PROJECT_INCLUDE` (runs after RC is
   enabled).
2. **find-root-path hygiene** (`toolchain.cmake`) — without `CMAKE_FIND_ROOT_PATH`
   + `*_MODE_*=ONLY`, `find_package()` finds *host* `/usr/include` and `/usr/lib`
   (e.g. host zlib), dragging host glibc headers into a windows-msvc build.

## Files

- `toolchain.cmake` — clang-cl/LLD + CRT/SDK includes, libpaths, find-root.
- `override.cmake` — `CMAKE_USER_MAKE_RULES_OVERRIDE`: clang-cl `/U`-path and
  `llvm-rc` `/D`→`-D` quirks (from cargo-xwin).
- `rc-sdk-includes.cmake` — `CMAKE_PROJECT_INCLUDE`: the RC-include fix.
- `build-deps.sh` — builds the deps in dependency order with the above.

## Use

```sh
# one-time: cache the MSVC CRT/SDK (any cargo-xwin build, or `xwin splat`)
cargo install cargo-xwin
# build the cross deps
packaging/windows-cross/build-deps.sh        # -> .build/prefix (gitignored)
```

Env: `XWIN_DIR` (CRT/SDK splat dir), `WINCROSS_PREFIX` (install prefix),
`BUILD_DIR`, `JOBS`. See the script header.
