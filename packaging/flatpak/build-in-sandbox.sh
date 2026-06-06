#!/usr/bin/env bash
# In-sandbox build for the flatpak (PR-9-2). Runs inside flatpak-builder
# with cwd = the checked-out source root, the GNOME 50 SDK + rust-stable
# + node22 extensions on PATH, and build-time network enabled.
#
# Why build in-sandbox at all: a host-built binary links the host's
# (Arch, bleeding-edge glibc) libraries and will not run against the
# older runtime glibc. Everything that ends up in /app must be built
# against the runtime.
set -euo pipefail

echo "::: 1/4 restore the OrcaSlicer deps prefix staged by the orca-deps module"
# The deps tree (Boost/CGAL/OCCT/…) is built by the separate orca-deps
# module and staged at /app/orca-deps. slic3r-ffi/build.rs looks for the
# prefix at this exact in-tree path, so restore it there.
prefix="external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local"
mkdir -p "${prefix}"
cp -a /app/orca-deps/. "${prefix}/"

echo "::: 2/4 build the app via the Tauri CLI"
npm ci
# Build through `tauri build`, NOT a bare `cargo build`. Tauri only
# serves the embedded production frontend (frontendDist) when built this
# way; a plain cargo build leaves the webview pointed at devUrl
# (http://localhost:1420), so the window loads whatever dev server is on
# that port instead of n3o-slic3r's UI. `tauri build` runs the frontend
# build (beforeBuildCommand) + the release cargo build and embeds dist/.
# --no-bundle: we do our own /app install, no deb/appimage.
./node_modules/.bin/tauri build --no-bundle

echo "::: 3/4 (backend built by tauri build above)"

echo "::: 4/4 install into /app"
install -Dm755 target/release/n3o-slic3r /app/bin/n3o-slic3r

# slic3r-ffi/build.rs builds the FFI via cmake into
# build/slic3r-ffi/<config>/ (Ninja Multi-Config); config is set to
# Release for the shipped artifact (see the app module's env).
so="$(find build/slic3r-ffi -name 'libslic3r_ffi.so.0' -print -quit)"
if [[ -z "${so}" ]]; then
  echo "error: libslic3r_ffi.so.0 not found under build/slic3r-ffi" >&2
  find build -name 'libslic3r_ffi.so*' >&2 || true
  exit 1
fi
install -Dm755 "${so}" /app/lib/libslic3r_ffi.so.0
ln -sf libslic3r_ffi.so.0 /app/lib/libslic3r_ffi.so
# The binary's rpath points at the (build-time) OUT_DIR, which is gone
# at runtime; /app/lib is on the runtime's default loader path, so the
# soname `libslic3r_ffi.so.0` resolves there.

# Bundled resources (profiles + plugins). The app reads these from
# $N3O_SLIC3R_RESOURCES_ROOT (set in finish-args).
res=/app/share/n3o-slic3r/resources
mkdir -p "${res}"
cp -r resources/profiles "${res}/profiles"
cp -r resources/plugins "${res}/plugins"

# Desktop integration: launcher, icon, and AppStream metadata (the last
# is required for the bundle to be considered a valid app).
appid=org.thegraveyard.n3o-slic3r
install -Dm644 "packaging/flatpak/${appid}.desktop" "/app/share/applications/${appid}.desktop"
install -Dm644 src-tauri/icons/128x128.png "/app/share/icons/hicolor/128x128/apps/${appid}.png"
install -Dm644 "packaging/flatpak/${appid}.metainfo.xml" "/app/share/metainfo/${appid}.metainfo.xml"

echo "::: done"
