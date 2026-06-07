# Windows cross-compilation (Linux → x86_64-pc-windows-msvc)

Goal: build n3o-slic3r for Windows **entirely on Linux** — no Windows host, no
wine — producing real **MSVC-ABI** binaries (so they link against the
`cl.exe`-built world and use WebView2 the way Tauri expects).

Toolchain: **clang-cl + LLD** in MSVC mode, against the MSVC CRT + Windows SDK
fetched by **[cargo-xwin](https://github.com/rust-cross/cargo-xwin)** (which
wraps [`xwin`](https://github.com/Jake-Shadle/xwin)). This is the same ABI a
native MSVC build produces.

## Status (2026-06-07) — `libslic3r.lib` cross-built

**The OrcaSlicer slicing engine itself cross-compiles** to a windows-msvc COFF
static archive — `libslic3r.lib`, 255/255 objects, `Machine:
IMAGE_FILE_MACHINE_AMD64`. Both the full dependency tree and the engine source
build under clang-cl + LLD with no Windows host and no wine.

The dependency tree (each a full Ninja build → `.lib`):

| dep | result |
|-----|--------|
| **OCCT** 7.6.0 | full STEP module set (the scariest dep) ✅ |
| **TBB** 2021.5.0 · **zlib** 1.3.1 · **OpenEXR/IlmBase** 2.5.5 | clean ✅ |
| **Boost** 1.84.0 | clean via Boost's own CMake build (not `b2`) ✅ |
| **OpenVDB** · **Blosc** (Orca forks) | link `libopenvdb.lib` / `libblosc.lib` ✅ |
| **NLopt** 2.5.0 · **Qhull** 8.0.2 · **libnoise** · **draco** 1.5.7 | clean ✅ |
| **Cereal** 1.3.0 · **Eigen** 5.0.1 · **CGAL** 5.6.3 | header-only install ✅ |
| **libpng** 1.6.35 · **FreeType** 2.12.1 · **GLFW** 3.4 · **libjpeg-turbo** 3.0.1 | clean ✅ |
| **OpenCV** 4.6.0 (`world`: core+imgproc+imgcodecs+highgui) | links `opencv_world460.lib` ✅ |
| **expat** 2.2 (Orca's bundled) | compiled to satisfy `find_package` ✅ |
| **GMP** 6.2.1 · **MPFR** 4.2.2 | OrcaSlicer's vendored MSVC prebuilts (copy) ✓ |
| **OpenSSL** 1.1.1w · **CURL** 7.75 | headers + stub libs — *interim*, see below |

**GMP/MPFR don't cross** (configure + assembly, no CMake). OrcaSlicer ships
**prebuilt MSVC** import libs + DLLs for them — those are MSVC-ABI, so clang-cl
links them directly; `build-deps.sh` just copies them in.

**OpenSSL/CURL are stubbed for the libslic3r *compile* only.** libslic3r
`#include`s just `<openssl/md5.h>` and, as a *static* archive, links neither;
CURL it doesn't use at all. Real OpenSSL/CURL MSVC cross-builds are needed for
the FFI-shim **DLL link** — that's the next step, not done here.

**One source patch** — `patches/0001-AABBTreeLines-eigen-cast-conformance.patch`.
The single source-level clang-cl-vs-cl.exe difference across the whole engine
(17 instantiations of one call site): MSVC's permissive mode binds a lazy Eigen
`.cast<>()` expression to a `Matrix` parameter; clang-cl (conformant) won't.
One-line fix, applied to the submodule at build time, tree left pinned.

**Remaining:** the FFI shim → `slic3r_ffi.dll` (real OpenSSL/CURL + the
`build.rs` Windows branch) + `src-tauri` via cargo-xwin + the Tauri NSIS bundle.
No compiler walls and no dep walls remain — what's left is our own integration.

Boost note: skip `b2` entirely — Boost 1.84 ships a CMake build that reuses this
same toolchain. It only *installs* headers for the compiled libs you select, so
`build-deps.sh` also drops the complete pre-assembled `boost/` header tree into
the prefix (header-only boost like `any`/`interprocess` that OpenVDB pulls).

### Why not OrcaSlicer's own Windows deps build

`external/OrcaSlicer/deps/deps-windows.cmake` is a **native-Windows** superbuild
— Visual Studio generator + `msbuild … INSTALL.vcxproj` per dep. CMake's VS
generators only exist on a Windows-host CMake, so it does not cross. This
directory rebuilds each dep through **Ninja + clang-cl** instead, reusing
OrcaSlicer's version pins + patches (`deps/<Name>/`).

## Six toolchain gaps we had to fix

cargo-xwin's generated toolchain targets *cargo*, not a CMake deps superbuild
(let alone the engine), so six things were missing (all baked into the files
here):

1. **RC includes** (`rc-sdk-includes.cmake`) — CMake preprocesses `.rc` files
   with `clang-cl -E`, which pulls includes from the *target* dirs, not the C/C++
   `/imsvc` flags. Version-info `.rc` files therefore failed with
   `'windows.h' file not found`. Fix: add the SDK include dirs via
   `include_directories()` from `CMAKE_PROJECT_INCLUDE` (runs after RC is
   enabled).
2. **find-root-path hygiene** (`toolchain.cmake`) — without `CMAKE_FIND_ROOT_PATH`
   + `*_MODE_*=ONLY`, `find_package()` finds *host* `/usr/include` and `/usr/lib`
   (e.g. host zlib), dragging host glibc headers into a windows-msvc build.
3. **release-CRT only** (`toolchain.cmake`) — xwin ships only the *release* MSVC
   CRT (no `msvcrtd.lib`). A dep with an old `cmake_minimum` leaves policy
   `CMP0091` OLD, so `CMAKE_MSVC_RUNTIME_LIBRARY` is ignored and a Debug
   try-compile emits `/MDd` → `lld-link: could not open 'msvcrtd.lib'` (hit on
   OpenEXR). Fix: `CMAKE_POLICY_DEFAULT_CMP0091 NEW` + force the release runtime
   for every config.
4. **clang-19 legacy-C errors** (`toolchain.cmake`) — clang 19 promoted
   `incompatible-pointer-types` / `implicit-function-declaration` /
   `int-conversion` to hard errors; old C (boost.container's dlmalloc) trips
   them. Fix: downgrade those three back to warnings for C — the clang analogue
   of OrcaSlicer's `-fpermissive` GCC workaround.
5. **llvm-rc codepage** (`llvm-rc-cp1252` wrapper, wired as `CMAKE_RC_COMPILER`)
   — llvm-rc rejects a Latin-1 `©` in a non-Unicode `VERSIONINFO`
   (`Non-ASCII 8-bit codepoint (169) …`), hit on OCCT and FreeType. rc.exe
   assumes the system codepage; the wrapper sets `/C 1252`. Only the rc step
   uses `CMAKE_RC_COMPILER`, so the preprocess (clang-cl) is unaffected.
6. **`-Werror` vs clang-cl strictness** (`clang-cl-nowerror` launcher, wired as
   `CMAKE_<LANG>_COMPILER_LAUNCHER`) — bundled deps (clipper2, …) compile with
   `/WX`/`-Werror` and are clean under cl.exe but not clang-cl's wider warning
   set. The launcher inserts `-Wno-error` just before `-c` (after the target's
   `/WX`, before `override.cmake`'s `-c -- <src>` positional zone) so warnings
   stay warnings — matching the cl.exe build.

## Files

- `toolchain.cmake` — clang-cl/LLD + CRT/SDK includes, libpaths, find-root,
  release-runtime, legacy-C, the rc wrapper + no-werror launcher, module stub.
- `override.cmake` — `CMAKE_USER_MAKE_RULES_OVERRIDE`: clang-cl `/U`-path and
  `llvm-rc` `/D`→`-D` quirks (from cargo-xwin).
- `rc-sdk-includes.cmake` — `CMAKE_PROJECT_INCLUDE`: the RC-include fix.
- `llvm-rc-cp1252` — `CMAKE_RC_COMPILER` wrapper: cp1252 input codepage (gap 5).
- `clang-cl-nowerror` — `CMAKE_<LANG>_COMPILER_LAUNCHER`: insert `-Wno-error`
  before `-c` (gap 6).
- `cmake-stubs/InstallRequiredSystemLibraries.cmake` — no-op shadow for the
  builtin module whose MSVC branch queries the Windows registry (can't cross).
  Prepended to `CMAKE_MODULE_PATH` (survives a dep's `list(APPEND …)`, e.g.
  OrcaSlicer's top-level CMake); `build-deps.sh` also copies it into a dep's own
  `cmake/` dir when the dep *replaces* the module path (c-blosc).
- `patches/0001-AABBTreeLines-eigen-cast-conformance.patch` — the one source
  conformance fix, applied to the submodule at build time.
- `build-deps.sh` — builds the deps in dependency order, stubs OpenSSL/CURL,
  and applies the patch (`patch_orca`).

## Use

```sh
# one-time: cache the MSVC CRT/SDK (any cargo-xwin build, or `xwin splat`)
cargo install cargo-xwin
# build the cross deps
packaging/windows-cross/build-deps.sh        # -> .build/prefix (gitignored)
```

Env: `XWIN_DIR` (CRT/SDK splat dir), `WINCROSS_PREFIX` (install prefix),
`BUILD_DIR`, `JOBS`. See the script header.
