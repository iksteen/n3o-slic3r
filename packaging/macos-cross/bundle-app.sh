#!/usr/bin/env bash
# Assemble a relocatable, ad-hoc-signed n3o-slic3r.app (and optionally a .dmg)
# for macOS from a Linux cross build. Tauri's own macOS bundler and `codesign`
# are macOS-only, so this replicates the bundle layout that
# src-tauri/tauri.macos.conf.json + tauri.conf.json describe and signs with
# rcodesign (the apple-codesign crate — runs on Linux).
#
# Prereqs:
#   - the app binary is cross-built:
#       ./build.sh <arch> cargo build -p n3o-slic3r --target <triple> --release
#   - the shim dylib exists at build/slic3r-ffi-<arch>/RelWithDebInfo/ (the cargo
#     build above produces it)
#   - rcodesign on PATH  (cargo install apple-codesign)
#   - for --dmg: genisoimage (cdrkit) + git/cmake/make/cc — libdmg-hfsplus's
#     `dmg` tool is built in-tree on first use (see the --dmg block below).
#
# Usage:  bundle-app.sh <arm64|x86_64> [--dmg]
set -euo pipefail

ARCH="${1:?usage: bundle-app.sh <arm64|x86_64> [--dmg]}"; shift || true
case "$ARCH" in
  arm64)  TRIPLE=aarch64-apple-darwin ;;
  x86_64) TRIPLE=x86_64-apple-darwin ;;
  *) echo "arch must be arm64 or x86_64" >&2; exit 2 ;;
esac
WANT_DMG=0; [ "${1:-}" = "--dmg" ] && WANT_DMG=1

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$here/../.." && pwd)"
cd "$REPO_ROOT"

APP_NAME="n3o-slic3r"
IDENTIFIER="org.thegraveyard.n3o-slic3r"
VERSION="$(grep -m1 '^version' src-tauri/Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
DEPLOY=11.3

BIN="target/$TRIPLE/release/$APP_NAME"
DYLIB="build/slic3r-ffi-$ARCH/RelWithDebInfo/libslic3r_ffi.0.1.0.dylib"
[ -f "$BIN" ]   || { echo "error: app binary not found: $BIN — cross-build it first" >&2; exit 1; }
[ -f "$DYLIB" ] || { echo "error: shim dylib not found: $DYLIB" >&2; exit 1; }
command -v rcodesign >/dev/null || { echo "error: rcodesign not on PATH (cargo install apple-codesign)" >&2; exit 1; }

OUT="target/$TRIPLE/release/bundle/macos"
APP="$OUT/$APP_NAME.app"
C="$APP/Contents"

echo ":: assembling $APP (v$VERSION, $ARCH)"
rm -rf "$APP"
mkdir -p "$C/MacOS" "$C/Frameworks" "$C/Resources"

# Executable + the engine dylib (binary already carries an
# @executable_path/../Frameworks rpath and links @rpath/libslic3r_ffi.0.dylib).
cp "$BIN" "$C/MacOS/$APP_NAME"
cp "$DYLIB" "$C/Frameworks/libslic3r_ffi.0.dylib"
chmod 0755 "$C/MacOS/$APP_NAME" "$C/Frameworks/libslic3r_ffi.0.dylib"

# Icon + bundled resources (same mapping as tauri.conf.json's bundle.resources).
cp src-tauri/icons/icon.icns "$C/Resources/icon.icns"
cp -R resources/profiles "$C/Resources/profiles"
cp -R resources/plugins  "$C/Resources/plugins"

cat > "$C/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>      <string>en</string>
  <key>CFBundleDisplayName</key>            <string>${APP_NAME}</string>
  <key>CFBundleExecutable</key>             <string>${APP_NAME}</string>
  <key>CFBundleIconFile</key>               <string>icon.icns</string>
  <key>CFBundleIdentifier</key>             <string>${IDENTIFIER}</string>
  <key>CFBundleName</key>                   <string>${APP_NAME}</string>
  <key>CFBundlePackageType</key>            <string>APPL</string>
  <key>CFBundleShortVersionString</key>     <string>${VERSION}</string>
  <key>CFBundleVersion</key>                <string>${VERSION}</string>
  <key>CFBundleSupportedPlatforms</key>     <array><string>MacOSX</string></array>
  <key>LSMinimumSystemVersion</key>         <string>${DEPLOY}</string>
  <key>NSHighResolutionCapable</key>        <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
</dict>
</plist>
PLIST

# Ad-hoc sign (no cert): signs the nested dylib + the main executable. The
# disable-library-validation entitlement lets the separately-signed engine dylib
# load (matches src-tauri/entitlements.macos.plist on the native build).
echo ":: ad-hoc signing with rcodesign"
rcodesign sign \
  --entitlements-xml-file src-tauri/entitlements.macos.plist \
  "$APP"

# rcodesign's `verify` is self-admittedly buggy on bundles; read the signature
# back instead and assert the main executable is ad-hoc signed. Authoritative
# validation (codesign -v / spctl) needs a Mac — and Gatekeeper still rejects an
# ad-hoc app on download (no Developer-ID notarization), the same caveat the
# native build carries.
echo ":: signature readback"
if rcodesign print-signature-info "$C/MacOS/$APP_NAME" 2>/dev/null \
     | grep -q 'CodeSignatureFlags(ADHOC)'; then
  echo ":: main executable is ad-hoc signed (entitlements embedded)"
else
  echo ":: WARNING: could not confirm an ad-hoc signature on the main executable" >&2
fi

echo ":: done -> $APP"

if [ "$WANT_DMG" = 1 ]; then
  # A macOS .dmg is a UDIF-wrapped filesystem image. Apple's hdiutil is
  # macOS-only and the legacy HFS that genisoimage can emit is no longer
  # mountable on current macOS — so build the volume as ISO9660 + Rock Ridge
  # (-r: POSIX perms, symlinks, long names) + Apple extensions (-apple), which
  # current macOS mounts as ISO9660, then wrap it in a compressed UDIF with
  # libdmg-hfsplus's `dmg`. (This is the same genisoimage→dmg path Bitcoin Core
  # uses for its cross-built macOS DMGs.) Validated end to end: hdiutil verify +
  # attach succeed and the .app stays codesign-valid on the mounted volume.
  command -v genisoimage >/dev/null || { echo "error: genisoimage not found (install cdrkit/cdrtools)" >&2; exit 1; }
  # The `dmg` tool (libdmg-hfsplus) wraps the raw image into a compressed UDIF.
  # Honor an explicit $DMG_TOOL or a `dmg` on PATH; otherwise build libdmg-hfsplus
  # in-tree, into the gitignored .build/ scratch, so the .dmg step is
  # self-contained (no dependency on a tool checked out under $HOME). Built once,
  # then reused.
  dmg_tool="${DMG_TOOL:-$(command -v dmg 2>/dev/null || true)}"
  if [ -z "$dmg_tool" ] || [ ! -x "$dmg_tool" ]; then
    src="$here/.build/libdmg-hfsplus"; dmg_tool="$src/dmg/dmg"
    if [ ! -x "$dmg_tool" ]; then
      for t in git cmake make cc; do command -v "$t" >/dev/null || { echo "error: building libdmg-hfsplus needs '$t'" >&2; exit 1; }; done
      echo ":: building libdmg-hfsplus in-tree ($src)"
      [ -d "$src/.git" ] || git clone --depth 1 https://github.com/fanquake/libdmg-hfsplus "$src" >/dev/null 2>&1 \
        || { echo "error: git clone libdmg-hfsplus failed" >&2; exit 1; }
      ( cd "$src" && cmake . -DCMAKE_BUILD_TYPE=Release && make -j"$(nproc)" ) >"$src/build.log" 2>&1 \
        || { echo "error: libdmg-hfsplus build failed — see $src/build.log" >&2; tail -15 "$src/build.log" >&2; exit 1; }
    fi
    [ -x "$dmg_tool" ] || { echo "error: no usable dmg tool at $dmg_tool after build" >&2; exit 1; }
  fi
  DMGDIR="target/$TRIPLE/release/bundle/dmg"
  OUTDMG="$DMGDIR/$APP_NAME.dmg"
  STAGE="$(mktemp -d)"; CONTENT="$STAGE/content"; mkdir -p "$CONTENT" "$DMGDIR"
  # What the mounted volume shows: the .app + a drag-to-install Applications
  # symlink. cp -a preserves perms + the _CodeSignature.
  cp -a "$APP" "$CONTENT/"
  ln -s /Applications "$CONTENT/Applications"
  echo ":: building DMG volume image"
  genisoimage -quiet -no-cache-inodes -D -l -probe -V "$APP_NAME" -no-pad -r \
    -dir-mode 0755 -apple -o "$STAGE/raw.img" "$CONTENT"
  echo ":: compressing to UDIF (.dmg)"
  "$dmg_tool" "$STAGE/raw.img" "$OUTDMG"
  rm -rf "$STAGE"
  echo ":: dmg -> $OUTDMG ($(du -h "$OUTDMG" | cut -f1))"
fi
