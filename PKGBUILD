# Maintainer: Ingmar Steen <iksteen@gmail.com>
#
# Builds n3o-slic3r from the working tree this PKGBUILD lives in.
# Run `makepkg -s` from the repo root.
#
# Prerequisites the host has to satisfy *before* `makepkg`:
#   - The OrcaSlicer deps tree (Boost/CGAL/OCCT/TBB/OpenVDB/...) must
#     be built once: `./scripts/build.sh deps` (~17 min, ~3 G).
#     prepare() symlinks it into the build root so makepkg reuses it
#     instead of rebuilding from scratch under fakeroot.
#   - `npm` for the frontend, `cargo`/`cmake`/`ninja` for the FFI shim
#     and Tauri binary. All declared in makedepends.
#
# Build root note: this PKGBUILD does NOT use the default $srcdir
# because the repo's top-level `src/` (Vite source) collides with
# makepkg's default `$startdir/src`. A previous attempt to rsync the
# working tree into $srcdir created infinite recursion. The build
# root lives under /tmp/ instead (overridable via $BUILDDIR).
#
# The package payload:
#   - /usr/bin/n3o-slic3r
#   - /usr/lib/libslic3r_ffi.so{,.0}
#   - /usr/lib/<bundle-id>/resources/profiles/vendor/   (Tauri's deb)
#   - /usr/share/applications/n3o-slic3r.desktop
#   - /usr/share/icons/hicolor/.../n3o-slic3r.png
#   - /usr/share/licenses/n3o-slic3r/LICENSE

pkgname=n3o-slic3r
pkgver=0.1.0
pkgrel=1
pkgdesc="Modern desktop slicer UI driving libslic3r via orca-slicer-ffi"
arch=('x86_64')
url="https://github.com/iksteen/n3o-slic3r"
license=('AGPL-3.0-or-later')
depends=(
    'webkit2gtk-4.1'
    'gtk3'
    'libsoup3'
    'librsvg'
)
makedepends=(
    'rust'
    'nodejs'
    'npm'
    'cmake'
    'ninja'
    'git'
    'pkgconf'
    'rsync'
)
options=(
    # libslic3r's build sets its own optimization flags; don't let
    # makepkg's default `lto` slow it down further or risk breakage.
    '!lto'
)
source=()
sha256sums=()

# Build root lives OUTSIDE $startdir so the rsync source tree can't
# possibly contain its own target. NOT `${BUILDDIR:-...}` — makepkg
# defaults $BUILDDIR to $startdir when makepkg.conf doesn't set it,
# which is exactly the trap we're avoiding. Override with the env var
# below for parallel builds.
_buildroot="${N3O_PKGBUILD_BUILDROOT:-/tmp/n3o-slic3r-build}"

prepare() {
    # Refuse to run if someone aimed _buildroot inside the source
    # tree — same recursion trap that bit the first attempt.
    case "$_buildroot" in
        "$startdir"|"$startdir"/*)
            echo "error: _buildroot ($_buildroot) must be outside \$startdir ($startdir)" >&2
            return 1
            ;;
    esac

    rm -rf "$_buildroot"
    mkdir -p "$_buildroot"

    # Mirror the working tree. Skips caches + the heavy external deps
    # build dir; the latter is symlinked back in so cmake reuses the
    # prebuilt boost/cgal/occt/tbb tree.
    rsync -a \
        --exclude='/target/' \
        --exclude='/node_modules/' \
        --exclude='/build/' \
        --exclude='/dist/' \
        --exclude='/.git/' \
        --exclude='/external/OrcaSlicer/deps/build/' \
        --exclude='/external/OrcaSlicer/build/' \
        "$startdir/" "$_buildroot/"

    local deps_src="$startdir/external/OrcaSlicer/deps/build"
    if [[ ! -d "$deps_src/OrcaSlicer_dep/usr/local" ]]; then
        echo "error: OrcaSlicer deps tree missing — run \`./scripts/build.sh deps\` first" >&2
        return 1
    fi
    mkdir -p "$_buildroot/external/OrcaSlicer/deps"
    ln -sfn "$deps_src" "$_buildroot/external/OrcaSlicer/deps/build"
}

build() {
    cd "$_buildroot"

    # Frontend toolchain + the locally-pinned @tauri-apps/cli.
    npm ci --no-audit --no-fund

    # `tauri build` runs `npm run build` (vite → dist/), then `cargo
    # build --release` for the desktop binary (which in turn drives
    # the cmake build of libslic3r_ffi.so via the slic3r-ffi crate's
    # build.rs), then the .deb bundler. We extract that .deb in
    # package() so the Tauri-generated .desktop + icons + bundled
    # resource layout come along for free.
    #
    # `Release` instead of the build.rs default `RelWithDebInfo` —
    # makepkg's strip pass would drop the debug info anyway, and the
    # smaller .so saves a fakeroot-time roundtrip.
    export NODE_ENV=production
    export N3O_SLIC3R_FFI_CMAKE_CONFIG=Release
    npx --no-install tauri build --bundles deb
}

package() {
    cd "$_buildroot"

    # Locate the produced .deb. Tauri 2 names it `<productName>_<ver>_<arch>.deb`.
    # The Cargo workspace puts `target/` at the workspace root (not under
    # `src-tauri/`), so Tauri's bundler ends up here.
    local deb
    deb=$(find target/release/bundle/deb \
        -maxdepth 1 -type f -name '*.deb' -print -quit)
    if [[ -z "$deb" ]]; then
        echo "error: tauri deb bundle missing under target/release/bundle/deb" >&2
        return 1
    fi

    # Crack the .deb (ar archive of debian-binary + control.tar + data.tar).
    local stage="$_buildroot/.deb-stage"
    rm -rf "$stage" && mkdir -p "$stage"
    bsdtar -xf "$deb" -C "$stage"
    bsdtar -xf "$stage"/data.tar.* -C "$pkgdir"

    # libslic3r_ffi.so.0 is the FFI shim that the binary dlopens at
    # runtime. Tauri's bundler ignores it (not a declared resource),
    # and the rpath baked into the binary points at the cmake build
    # dir on the *build* host — which doesn't exist on the install
    # host. Installing the .so to /usr/lib/ makes ld.so find it
    # through the standard cache, regardless of the stale rpath.
    install -d "$pkgdir/usr/lib"
    install -Dm755 \
        "build/slic3r-ffi/Release/libslic3r_ffi.so" \
        "$pkgdir/usr/lib/libslic3r_ffi.so.0"
    ln -sfn libslic3r_ffi.so.0 "$pkgdir/usr/lib/libslic3r_ffi.so"

    install -Dm644 LICENSE.txt \
        "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}

# vim: set ts=4 sw=4 et ft=PKGBUILD:
