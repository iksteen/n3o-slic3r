# macOS cross-compile (Linux → \*-apple-darwin) via osxcross

Build the libslic3r engine, the FFI shim dylib, and the full Tauri app binary
for macOS **from a Linux host**, using [osxcross](https://github.com/tpoechtrager/osxcross)
(clang + cctools/ld64 + a packaged macOS SDK). This mirrors the
`packaging/windows-cross/` approach: OrcaSlicer's own `deps-macos.cmake`
superbuild does not cross (it bootstraps a host `b2` for Boost and assumes a
native Apple toolchain), so `build-deps.sh` rebuilds each dependency with the
osxcross toolchain into the **same** arch-namespaced prefix the native macOS
build uses, and `crates/slic3r-ffi/build.rs` drives the engine + shim build
through the osxcross toolchain when it detects a macOS target on a non-macOS
host.

Validated for **arm64** (SDK 15.4, target `arm64-apple-darwin`). The script
takes `x86_64` too; the two are independent (separate prefix, separate cargo
target).

## One-time: build osxcross with a packaged SDK

```bash
git clone https://github.com/tpoechtrager/osxcross.git ~/osxcross
# Package a macOS SDK from a Mac you have access to (here: CommandLineTools' SDK):
ssh mac 'tar -C /Library/Developer/CommandLineTools/SDKs -cf - MacOSX15.4.sdk' \
  | xz -T0 -3 > ~/osxcross/tarballs/MacOSX15.4.sdk.tar.xz
cd ~/osxcross && UNATTENDED=1 ./build.sh
```

This produces `~/osxcross/target/bin/{arm64,x86_64}-apple-darwin<NN>-clang` and
`~/osxcross/target/toolchain.cmake`. Override the location with `OSXCROSS_ROOT`
(default `~/osxcross/target`).

Host packages (Arch): `clang lld llvm cmake ninja curl unzip git`.

## Step 1 — cross-build the dependency tree

```bash
./build-deps.sh arm64        # ~30–45 min cold; installs ~88 static libs
```

Installs into `external/OrcaSlicer/deps/build/arm64/OrcaSlicer_dep/usr/local`
(the path `build.rs`'s macOS branch already expects). Per-dep stamps under
`<prefix>/.stamps/` make a re-run resume after a fixed failure instead of
rebuilding the whole tree; delete a stamp (or the prefix) to force a rebuild.
The completion marker `<prefix>/.deps-complete` is written only on a full
success.

## Step 2 — cross-build the FFI shim and/or the app

The `build.sh` wrapper exports the osxcross env three consumers read (the cmake
toolchain via `OSXCROSS_*`/`MACCROSS_PREFIX`, cargo's target linker, and the
`cc` crate's `CC_/CXX_/AR_` vars), then runs the command you give it:

```bash
# the engine + shim dylib (libslic3r builds cold ~15 min):
./build.sh arm64 cargo build -p slic3r-ffi --target aarch64-apple-darwin --release
# the full app binary (links the shim + the whole Tauri crate graph):
./build.sh arm64 cargo build -p n3o-slic3r  --target aarch64-apple-darwin --release
```

Output:
- `build/slic3r-ffi-arm64/RelWithDebInfo/libslic3r_ffi.0.1.0.dylib` — arm64
  Mach-O shim (install_name `@rpath/libslic3r_ffi.0.dylib`). `build.rs` also
  repoints the static `build/slic3r-ffi-current` symlink at the arch it built,
  the same as the native macOS path.
- `target/aarch64-apple-darwin/release/n3o-slic3r` — arm64 Mach-O app binary,
  linking `@rpath/libslic3r_ffi.0.dylib`.

## Notes / non-obvious bits

- **SDK find-mode host leak.** `toolchain.cmake` relaxes osxcross's find-root
  modes to `BOTH` so each dep finds the ones built before it. The cost is that
  `find_package` can also see *host Linux* libraries that aren't in the macOS
  sysroot; `build-deps.sh` disables the affected optional features explicitly
  (Boost.Iostreams bzip2/lzma/zstd, Boost.Locale ICU, OpenCV OpenEXR). A dep we
  actually provide is always found in the prefix first.
- **GMP/MPFR** cross-build from source with autotools (`--host=aarch64-apple-darwin…`),
  unlike the Windows path which reuses MSVC prebuilts.
- **libpng** force-includes `<math.h>` (osxcross's SDK doesn't pre-define the
  guard libpng's Classic-Mac `<fp.h>` branch checks) and disables arm64 NEON
  (its run-time NEON check needs an OS-detection file osxcross has none of).
- **draco** wraps each CLI executable's libs in GNU `--start-group` whenever the
  compiler id is plain `Clang` (osxcross) — a native Mac reports `AppleClang`
  and skips it; ld64 rejects the flag. Patched to skip it on Apple, as native.
- **encoding-check** (an OrcaSlicer host build-tool that runs during the build)
  is disabled for the cross build via `-DSLIC3R_ENC_CHECK=OFF` — cross-built it
  is a Mach-O binary the Linux host can't execute.
- **`__isPlatformVersionAtLeast`** (the runtime helper `@available` lowers to,
  used by libslic3r's `MacUtils.mm`) is supplied by `ffi/macos_availability_shim.mm`,
  gated to this host/target combo (a native macOS build gets it from Apple's
  clang_rt and would otherwise hit a duplicate symbol).

## Step 3 — bundle the `.app`

`npm run tauri build` produces the bundle on macOS, but Tauri's macOS bundler
and `codesign` are macOS-only. `bundle-app.sh` replicates the bundle layout on
Linux and ad-hoc signs with [`rcodesign`](https://github.com/indygreg/apple-platform-rs)
(`cargo install apple-codesign`):

```bash
./bundle-app.sh arm64        # -> target/aarch64-apple-darwin/release/bundle/macos/n3o-slic3r.app
```

It copies the cross-built binary + `libslic3r_ffi.0.dylib` (into
`Contents/Frameworks`, which the binary's `@executable_path/../Frameworks` rpath
already targets), the icon, and the `profiles`/`plugins` resources, writes
`Info.plist`, and ad-hoc signs the nested dylib + main executable with the
`disable-library-validation` entitlement (so the separately-signed engine dylib
loads — same as the native build).

Caveats:
- **Authoritative signature validation needs a Mac** (`codesign -v` / `spctl`).
  `rcodesign verify` is self-admittedly unreliable on bundles; the script reads
  the signature back instead and asserts an ad-hoc signature is present.
- **Gatekeeper still rejects an ad-hoc `.app` on download** (no Developer-ID
  notarization — needs a paid Apple account), the same caveat the native build
  carries.
- **`.dmg` is not built yet.** A macOS `.dmg` is an HFS+/APFS image; producing
  one on Linux needs `libdmg-hfsplus` (or `mkfs.hfsplus`). `bundle-app.sh --dmg`
  is stubbed for this follow-up.
