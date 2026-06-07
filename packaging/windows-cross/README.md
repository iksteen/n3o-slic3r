# Windows cross-compilation (Linux → x86_64-pc-windows-msvc)

Goal: build n3o-slic3r for Windows **entirely on Linux** — no Windows host, no
wine — producing real **MSVC-ABI** binaries (so they link against the
`cl.exe`-built world and use WebView2 the way Tauri expects).

Toolchain: **clang-cl + LLD** in MSVC mode, against the MSVC CRT + Windows SDK
fetched by **[cargo-xwin](https://github.com/rust-cross/cargo-xwin)** (which
wraps [`xwin`](https://github.com/Jake-Shadle/xwin)). This is the same ABI a
native MSVC build produces.

## Status (2026-06-07) — Windows installer cross-builds from Linux

**The whole app cross-builds to a distributable, on Linux, no Windows host and no
wine.** End to end: the dependency tree (clang-cl + LLD) → the engine
(`libslic3r.lib`, 255/255 objects) → the FFI shim (`slic3r_ffi.dll` +
`slic3r_ffi.lib`, exporting the `slic3r_*` C API) → the FFI Rust crate and the
**full Tauri app** (`n3o-slic3r.exe`, 19M, `IMAGE_SUBSYSTEM_WINDOWS_GUI`, links
the FFI) via `cargo xwin` → the **NSIS installer**
(`n3o-slic3r_<ver>_x64-setup.exe`) with `slic3r_ffi.dll` bundled beside the exe.
Tauri 2's cross story is mature — no tauri-winres / WebView2 / resource-compiler
snags. See "Use" below for the three build steps.

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

**Three source patches** (`patches/`, applied to the pinned submodule at build
time) — the entire clang-cl-vs-cl.exe source delta across engine + shim:
- `0001-AABBTreeLines-…` — a lazy Eigen `.cast<>()` bound to a `Matrix`
  parameter (MSVC-permissive, clang-cl-conformant); materialise to the exact
  `Vec` type. 17 instantiations of one call site.
- `0002-psapi-lib-lowercase-…` — `Psapi.lib` → `psapi.lib`; lld-link resolves
  case-sensitively against the lowercased SDK splat.
- `0003-BoundingBox-explicit-construct-…` — clang-cl doesn't emit an inline
  private member template that MSVC does from a friend's explicit instantiation;
  instantiate it explicitly.

Plus, in the FFI crate's own `CMakeLists.txt` (Windows-guarded): `_USE_MATH_DEFINES`
+ `NOMINMAX` (the shim's headers use `M_PI`), and `WINDOWS_EXPORT_ALL_SYMBOLS`
(the `slic3r_*` C API has no `__declspec`, mirroring the Linux `.so`'s
export-all-public-symbols default). And **real MD5**: libslic3r's only OpenSSL
use, so `build-deps.sh` compiles OpenSSL's own `crypto/md5/*.c` into `libcrypto`
rather than cross-building all of OpenSSL.

**Remaining:** `src-tauri` via cargo-xwin (links `slic3r_ffi` through the import
lib — needs the `build.rs` Windows branch: drop the rpath, find the DLL/import
lib) + the Tauri NSIS bundle. The hard part — the C++ engine + shim — is done.

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
- `patches/000{1,2,3}-*.patch` — the clang-cl source deltas (Eigen cast, psapi
  case, explicit `construct` instantiation), applied to the submodule at build
  time by `patch_orca`.
- `build-deps.sh` — builds the deps in dependency order, builds real OpenSSL MD5
  + stubs libssl/CURL, adds Win32 system-lib case symlinks (`syscase`), and
  applies the patches (`patch_orca`).

## Use

```sh
# one-time: cache the MSVC CRT/SDK (cargo-xwin fetches it on first build)
cargo install cargo-xwin
# 1) build the cross deps  (-> .build/prefix, gitignored)
packaging/windows-cross/build-deps.sh
export WINCROSS_PREFIX=<.build/prefix from above>

# 2) the FFI crate / app  — build.rs drives the cross cmake for slic3r_ffi.dll
cargo xwin build --release --target x86_64-pc-windows-msvc -p n3o-slic3r

# 3) the installer  — tauri cross-bundles NSIS on Linux (cargo-xwin as runner).
#    ~/.cargo/bin must be on PATH so tauri can exec the cargo-xwin binary.
PATH="$HOME/.cargo/bin:$PATH" \
  npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis
#  -> target/x86_64-pc-windows-msvc/release/bundle/nsis/n3o-slic3r_<ver>_x64-setup.exe
```

Env: `XWIN_DIR` (CRT/SDK splat dir), `WINCROSS_PREFIX` (cross-deps prefix),
`BUILD_DIR`, `JOBS`. See the script header.

`slic3r_ffi.dll` ships **beside** the exe: the slic3r-ffi build script copies it
next to the binary (Windows resolves DLLs from the exe dir), and
`src-tauri/tauri.windows.conf.json` adds it as a root bundle resource so the NSIS
installer includes it. Installer signing is skipped on a Linux host (set
`bundle.windows.signCommand` for a custom signer).

## Publishing

`publish.sh` is the signed release path (parallel to `packaging/arch/publish.sh`
and `packaging/flatpak/publish.sh`): it cross-builds the installer via
`build-app.sh`, GPG-signs it with the project release key, and uploads the
`-setup.exe` + its detached `.sig` + the public key.

```sh
N3O_WIN_PUBLISH_DEST="user@host:/srv/www/n3o.thegraveyard.org/windows" \
  packaging/windows-cross/publish.sh
```

With `N3O_WIN_PUBLISH_DEST` unset it builds + signs and prints the manual upload
+ verify steps. Override the key with `N3O_WIN_GPG_KEY` and the served base URL
with `N3O_WIN_URL` (default `https://n3o.thegraveyard.org/windows`). This is GPG
signing for cross-channel verification (`gpg --verify`), **not** Windows
Authenticode — SmartScreen still prompts unless a `signCommand` cert is wired.
End users import the key once, then `gpg --verify <installer>.sig <installer>`
before running it.
