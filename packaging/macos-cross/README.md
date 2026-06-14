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

Validated for **arm64** (target `arm64-apple-darwin`). The script takes `x86_64`
too; the two are independent (separate prefix, separate cargo target).

## osxcross (built automatically, in-tree)

`ensure-osxcross.sh` builds osxcross on first use into the gitignored
`.build/osxcross/` (the same in-tree pattern as the `.dmg` tool) — **no Mac and
nothing in `$HOME` needed**. It pins osxcross to a commit and fetches a pinned,
checksummed **public macOS SDK** (15.5, from
[joseluisq/macosx-sdks](https://github.com/joseluisq/macosx-sdks)) into
osxcross's `tarballs/`, then runs `UNATTENDED=1 ./build.sh`. `build-deps.sh` and
`env.sh` call it to resolve the toolchain. Host packages (Arch):
`clang lld llvm cmake ninja curl unzip git`.

To reuse an existing install instead (e.g. `~/osxcross/target` or a
system/`/usr/local` one with a usable SDK), set `OSXCROSS_ROOT` — it takes
precedence over the in-tree build. To change SDK version, edit the pinned
`SDK_URL`/`SDK_SHA256` in `ensure-osxcross.sh`.

> The SDK is Apple's and not redistributable by Apple; the mirror is a community
> convenience — use it only if you hold a macOS license.

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

For the whole app artifact in one shot (deps + frontend + cross build + bundled
`.app`/`.dmg`), just use `./build.sh <arch>` (Step 3). To run individual cross
builds, the `env.sh` wrapper exports the osxcross env three consumers read (the
cmake toolchain via `OSXCROSS_*`/`MACCROSS_PREFIX`, cargo's target linker, and
the `cc` crate's `CC_/CXX_/AR_` vars), then runs the command you give it:

```bash
# the engine + shim dylib (libslic3r builds cold ~15 min):
./env.sh arm64 cargo build -p slic3r-ffi --target aarch64-apple-darwin --release
# the full app binary (links the shim + the whole Tauri crate graph):
./env.sh arm64 cargo build -p n3o-slic3r  --target aarch64-apple-darwin --release \
  --features custom-protocol
```

`--features custom-protocol` (a `src-tauri/Cargo.toml` alias for
`tauri/custom-protocol`) is **required** for the app. Tauri keys production vs.
dev off that feature (`is_dev() == !custom-protocol`): without it the binary runs
in dev mode and loads `build.devUrl` (`http://localhost:1420`) instead of the
embedded frontend — a white screen. `npm run tauri build` enables it
automatically; a plain `cargo build` does not.

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
./bundle-app.sh arm64          # -> …/bundle/macos/n3o-slic3r.app
./bundle-app.sh arm64 --dmg    # also -> …/bundle/dmg/n3o-slic3r.dmg
```

It copies the cross-built binary + `libslic3r_ffi.0.dylib` (into
`Contents/Frameworks`, which the binary's `@executable_path/../Frameworks` rpath
already targets), the icon, and the `profiles`/`plugins` resources, writes
`Info.plist`, and ad-hoc signs the nested dylib + main executable with the
`disable-library-validation` entitlement (so the separately-signed engine dylib
loads — same as the native build).

### `--dmg`

Needs **`genisoimage`** (cdrkit) and **libdmg-hfsplus**'s `dmg` tool. The `dmg`
tool is built automatically on first use — `bundle-app.sh` clones + builds
libdmg-hfsplus in-tree into `packaging/macos-cross/.build/libdmg-hfsplus/`
(gitignored, reused after) using `git cmake make` + a C/C++ compiler. To use an
existing build instead, set `$DMG_TOOL` or put a `dmg` on `PATH`.

The volume is built as **ISO9660 + Rock Ridge +
Apple extensions** (current macOS mounts it as ISO9660 — the legacy HFS that
`genisoimage` can emit is no longer mountable) and wrapped in a compressed UDIF
by `dmg`. The mounted volume shows the `.app` + a drag-to-`/Applications`
symlink. Validated on macOS 15: `hdiutil verify` + `attach` succeed, the inner
binary hashes identically to the source, and the `.app` stays `codesign -v
--strict` valid on the mounted volume. (Same `genisoimage`→`dmg` path Bitcoin
Core uses for its cross-built macOS DMGs.)

Caveats:
- **Authoritative signature validation needs a Mac** (`codesign -v` / `spctl`).
  `rcodesign verify` is self-admittedly unreliable on bundles; the script reads
  the signature back instead and asserts an ad-hoc signature is present.
- **Gatekeeper still rejects an ad-hoc `.app`/`.dmg` on download** (no
  Developer-ID notarization — needs a paid Apple account), the same caveat the
  native build carries. The DMG is a tidy container, not Gatekeeper acceptance.
- **The DMG is functional, not styled** — no background image or icon layout
  (those need a binary, undocumented `.DS_Store` that's impractical to forge on
  Linux). You get a clean volume with the app + the `/Applications` symlink.

## Step 4 — publish (one command)

```bash
npm run publish:mac-arm64      # or: publish:mac-x86_64
```

`publish.sh <arch>` is the release one-shot, mirroring the arch / flatpak /
windows channels: it ensures the cross-deps prefix (builds it on demand if
absent), cross-builds the app (`--features custom-protocol`), assembles + ad-hoc
signs the `.app` and `.dmg`, names the artifact `n3o-slic3r_<version>_<aarch64|x64>.dmg`,
**GPG-signs** it with the shared project release key, and uploads the `.dmg` + its
detached `.sig` + the public key via `rsync`.

Upload destination: `N3O_PUBLISH_DEST` — the site *base* (e.g.
`user@host:/srv/www/n3o.thegraveyard.org`); this channel uploads to `<dest>/pkg`,
shared with the arch/windows channels. The served base URL is `N3O_BASE_URL`
(default `https://n3o.thegraveyard.org`; this channel serves from `<base>/pkg`).
When `N3O_PUBLISH_DEST` is unset it prints the manual `rsync` + verify steps
instead. The GPG signature is for `gpg --verify` integrity, not Apple
notarization — Gatekeeper still prompts on download (right-click → Open).
