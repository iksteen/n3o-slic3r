# Road to 1.0 — post-MVP backlog (use-discovered)

Items surfaced *after* the MVP candidate (Phases 0–9 done, 2026-06-07),
mostly from the project lead using the app. **None block the MVP** — this
is the path to a canonical "1.0". The engineering notes are first-pass
reads to preserve the scoping context, not commitments. The *planned*
post-MVP deferrals (compose hook, hot reload, Orca preset importer, etc.)
live in `Execution_Plan.md` §16; this file is for what we found in use.

## Features (discovered in use, 2026-06-07)

### 1. Send progress bar
Driver-layer, self-contained. Send already uploads the sliced bundle
(Bambu over FTPS, U1 over Moonraker HTTP) but it's fire-and-forget from
`SendControls`. Add upload-progress callbacks in the driver → a progress
event → a bar at the send site. No scene impact. Smallest of the set.

### 2. Lay an object flat on the build plate
Transform op. Tractable version: **"place on face"** — click a face,
rotate its normal to −Z, drop to bed (we already have `rotate_object` +
the bed). True *auto*-flatten (most-stable resting face, no input) needs
face-area / convex-hull analysis + a face-pick UX (renderer raycast → a
Rust command). Independent of the plate features.

### 3. Auto-arrange on the plate, spilling to extra plates
Extends the existing `core/scene/arrange.rs` greedy pack, which already
*reports* overflow (`AutoArrangeOverflow` with the un-placed objects) but
doesn't act on it. 1.0: catch the overflow, add plate(s) bound to the
same printer, and place the spill there — built on #4's move primitive.

### 4. Send objects to another / a new plate
A `move_objects_to_plate(object_ids, target_or_new)` command + UI (drag
onto a plate tab, or a "send to plate" menu). The scene model was built
for this: per-plate objects with scene-wide mesh sharing, so a
move-between-plates op doesn't copy mesh buffers. **#4 underpins #3** —
build it first.

### 5. UI optimizations
TBD — to discuss. Placeholder; capture specifics here as they're named.

**Format note:** none of #3/#4 touch the frozen MVP format — per-plate
objects and extra plates already round-trip, so 1.0 plate features won't
reopen the format.

**Suggested order:** #4 → #3 (the plate cluster); #1 in parallel
(driver-only); #2 independent; then the UI items.

## Windows build — cross-compile feasibility (2026-06-07)

Goal: a Windows build, ideally **cross-compiled from Linux** (CI-friendly).
PRD §3.2 lists Windows as native + post-MVP; this revisits the *how*.

**Tauri cross-compiles fine** (an earlier "Tauri can't cross" claim was
wrong, Tauri-v1 instinct). `cargo-xwin` (clang-cl + the xwin-provided
Windows SDK/CRT) targets `x86_64-pc-windows-msvc` from Linux; the
WebView2 loader links through the `windows` crates; the **NSIS** bundler
(`makensis`) runs on Linux. (WiX/MSI is the Windows-host-only bit; NSIS
sidesteps it.)

**The crux is OrcaSlicer's C++ dependency tree** (Boost, TBB, CGAL,
OCCT, OpenVDB). Our Windows deps path (`external/OrcaSlicer/deps/
deps-windows.cmake` + `build_release_vs2022.bat`) is MSVC/Visual-Studio-
native (VS generator). Open question: do these build through clang-cl +
xwin + Ninja from Linux? Boost/TBB likely fine; **OCCT and OpenVDB are
the expected fighters** (Windows-specific code paths, transitive deps) —
unverified, don't assume either way.

**Our own code** is plausibly close: the FFI shim
(`crates/slic3r-ffi/build.rs`) is Linux-shaped (`.so`, `dylib`,
`-Wl,-rpath`) and needs a Windows platform-split — build `slic3r_ffi.dll`,
link via its import lib, drop the rpath, ship the DLL next to the `.exe`
(Windows resolves DLLs from the exe dir). With clang-cl that's a *cross*
path, not native-only.

**Decision (2026-06-07):** full cross — clang-cl/xwin, no Windows host, no
wine, no prebuilt deps. Backed by a feasibility spike.

**Spike result (2026-06-07).** Scaffold + findings committed at
`packaging/windows-cross/` (see its README). **The entire libslic3r dependency
tree cross-compiles clean** under clang-cl + LLD against the cargo-xwin MSVC
CRT/SDK: OCCT 7.6.0 (the scariest dep), TBB, zlib, OpenEXR/IlmBase, Boost 1.84
(via Boost's own CMake build, not `b2`), OpenVDB (`libopenvdb.lib`), Blosc
(`libblosc.lib`), NLopt, Qhull, and the header-only Cereal / Eigen / CGAL. The
two that can't cross — **GMP / MPFR** (configure + assembly, no CMake) — reuse
OrcaSlicer's *vendored* prebuilt MSVC import libs+DLLs (already MSVC-ABI, so
clang-cl links them directly). OrcaSlicer's own `deps-windows.cmake` is
VS-generator/msbuild-native and does *not* cross, so the scaffold drives each
dep through **Ninja + clang-cl**, reusing OrcaSlicer's version pins + patches.
Four toolchain gaps were found + fixed (all in the scaffold's README): the `.rc`
preprocess needed the SDK includes; `find_package` needed find-root-path hygiene
(else host `/usr` leaks in); xwin's release-only CRT needed `CMP0091 NEW` +
forced release runtime (else a Debug try-compile wants the absent
`msvcrtd.lib`); and clang-19's promoted legacy-C errors needed downgrading for
old C (boost.container's dlmalloc). Plus one per-dep quirk (c-blosc's CPack
`InstallRequiredSystemLibraries` queries the Windows registry) shadowed with a
no-op module.

**Update — `libslic3r.lib` cross-built (2026-06-07).** Past the deps, **the
engine itself now cross-compiles**: `libslic3r.lib`, 255/255 objects,
`IMAGE_FILE_MACHINE_AMD64`. Two more general toolchain pieces were needed (a
cp1252 `llvm-rc` wrapper for `©` in OCCT/FreeType version resources, and a
`clang-cl-nowerror` launcher so bundled deps' `/WX` doesn't turn clang's wider
warning set fatal), plus the libslic3r deps (libpng, FreeType, GLFW, expat,
libnoise, libjpeg-turbo, draco, OpenCV `world`). The **only** source-level
clang-cl-vs-cl.exe difference in the whole engine was a single call site (17
instantiations) where MSVC's permissive mode bound a lazy Eigen `.cast<>()`
expression to a `Matrix` param — fixed by one line
(`patches/0001-AABBTreeLines-…`, applied to the submodule at build time). So the
engine is conformant-clean bar one line: **no compiler wall, no porting effort.**

**Update — `slic3r_ffi.dll` cross-built (2026-06-07).** The FFI shim links too:
a PE32+ x86-64 DLL + import lib exporting the `slic3r_*` C API. The shim needed
three small things (in the FFI crate's `CMakeLists.txt`, Windows-guarded):
`_USE_MATH_DEFINES`/`NOMINMAX` (its headers use `M_PI`), and
`WINDOWS_EXPORT_ALL_SYMBOLS` (the C API carries no `__declspec`, matching the
Linux `.so`). The DLL *link* surfaced the expected real-symbol needs, all narrow:
**real MD5** (libslic3r's only OpenSSL use — compile OpenSSL's own
`crypto/md5/*.c`, no full-OpenSSL cross), two more clang-cl source patches
(`Psapi.lib` case, an explicit `construct` instantiation), and a couple of Win32
system-lib case symlinks. **The entire native C++ side — engine + shim — now
cross-builds and links.**

**Update — the FFI Rust crate cross-builds via `cargo xwin` (2026-06-07).** The
`build.rs` Windows branch drives the cross cmake + applies the patches, and
`cargo xwin build --target x86_64-pc-windows-msvc` links a real windows `.exe`
(`introspect.exe`) that imports `slic3r_ffi.dll`. One binding wrinkle: the
`SLIC3R_SCOPE_*`/`SLIC3R_OPT_*` enum constants are `u32` under GCC but `i32`
under MSVC (C leaves an all-non-negative enum's signedness to the compiler), so
the wrapper casts `as u32` at those two boundaries (lossless — small positive
bitflags). Linux build + the 26 FFI tests stay green.

**Update — the full app + NSIS installer cross-build (2026-06-07).** `src-tauri`
cross-compiled with no Tauri/WebView2/resource-compiler snags (Tauri 2's cross
story is mature): `n3o-slic3r.exe` (19M, GUI subsystem, imports `slic3r_ffi.dll`)
via `cargo xwin`, and the **NSIS installer** (`n3o-slic3r_<ver>_x64-setup.exe`)
via `tauri build --runner cargo-xwin --bundles nsis` — makensis runs on Linux.
The only wiring needed: the src-tauri rpath guard, and a
`tauri.windows.conf.json` adding `slic3r_ffi.dll` as a root bundle resource so it
ships beside the exe (the loader resolves DLLs from the exe dir; the slic3r-ffi
build script also copies it there for `cargo`-built binaries). Installer signing
is skipped on a Linux host (`bundle.windows.signCommand` for a custom signer).

**Windows cross-build: complete from Linux** — deps → engine → FFI DLL → app →
installer, no Windows host, no wine. Post-MVP polish left: wire it into CI, add a
`build-app.sh` driver (mirroring the flatpak/arch publish scripts), and an
authenticode signer. None are unknowns.

**Fallback** if the deps cross stalls: a native Windows CI runner
(`windows-latest` + MSVC), lifting OrcaSlicer's `build_release_vs2022.bat` /
`deps-windows.cmake`. The spike says we shouldn't need it.

## Other pending (not 1.0 features, just TODOs)

- **Arch self-hosted publish — verify end-to-end.**
  `packaging/arch/publish.sh` (+ README section) builds a signed
  `.pkg.tar.zst` for bare `pacman -U`. Written, `bash -n`-clean, and the
  package-name resolution is confirmed — but the full `makepkg` build and
  the upload are **untested**; run it on a real Arch box (after the
  flatpak build) to confirm.
- **`docs/site/` → its own repository.** The landing page currently lives
  untracked under `docs/site/`; it's slated to move out to a dedicated
  repo.
