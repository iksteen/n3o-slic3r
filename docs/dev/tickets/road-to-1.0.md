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
`packaging/windows-cross/` (see its README). OCCT 7.6.0 (the scariest dep),
TBB, zlib, and OpenEXR/IlmBase all cross-compile **clean** under clang-cl + LLD
against the cargo-xwin MSVC CRT/SDK — heavy templated C++ is not a wall.
OrcaSlicer's own `deps-windows.cmake` is VS-generator/msbuild-native and does
*not* cross, so the scaffold drives each dep through **Ninja + clang-cl**,
reusing OrcaSlicer's version pins + patches. Two gaps the cargo-xwin toolchain
lacked for a CMake deps superbuild were found + fixed: the `.rc` preprocess
needed the SDK includes, and `find_package` needed find-root-path hygiene (else
host `/usr` headers/libs leak into a windows-msvc build).

**Remaining (reproducible via `packaging/windows-cross/build-deps.sh`):**
Boost (`b2` clang-win cross — the one fiddly dep) → unblocks OpenVDB's own
compile; the remaining OrcaSlicer deps (CGAL / Cereal / Eigen / NLopt / Qhull /
…); then libslic3r + the FFI shim's `build.rs` Windows branch (`.dll` + import
lib, drop rpath) + the Tauri MSI/NSIS bundle.

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
