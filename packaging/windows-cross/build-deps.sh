#!/usr/bin/env bash
# Cross-build OrcaSlicer's libslic3r dependency tree to x86_64-pc-windows-msvc
# from Linux — clang-cl + LLD + the MSVC CRT/SDK (cargo-xwin), driven through
# Ninja (OrcaSlicer's own deps-windows.cmake is VS-generator/msbuild-native and
# does NOT cross; this rebuilds each dep with the toolchain in this directory).
#
# STATUS (2026-06-07): validated cross-clean — zlib, TBB, OpenEXR/IlmBase, OCCT
# (C++; .rc fixed). WIP — Boost (b2 cross), OpenVDB (needs Boost), and the
# remaining OrcaSlicer deps (CGAL, Cereal, Eigen, NLopt, Qhull, …) which follow
# the same `dep` pattern; pins live in external/OrcaSlicer/deps/<Name>/.
#
# Prereqs (Arch pkgs): clang lld llvm cmake ninja  + `cargo install cargo-xwin`
#   then run cargo-xwin once (any windows-msvc build) so the CRT/SDK are cached.
#
# Env:
#   XWIN_DIR         CRT+SDK splat dir.  Default: cargo-xwin's cache.
#   BUILD_DIR        scratch (sources + builds).  Default: ./.build
#   WINCROSS_PREFIX  install prefix for the cross deps.  Default: $BUILD_DIR/prefix
#   JOBS             parallelism.  Default: nproc
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XWIN_DIR:=${XDG_CACHE_HOME:-$HOME/.cache}/cargo-xwin/xwin}"
: "${BUILD_DIR:=$here/.build}"
: "${WINCROSS_PREFIX:=$BUILD_DIR/prefix}"
: "${JOBS:=$(nproc)}"
export XWIN_DIR WINCROSS_PREFIX
SRC="$BUILD_DIR/src"

[[ -d "$XWIN_DIR/crt/include" && -d "$XWIN_DIR/sdk/include" ]] || {
  echo "error: XWIN_DIR=$XWIN_DIR has no crt/sdk. Run a cargo-xwin build once," >&2
  echo "       or 'xwin --accept-license splat --output $XWIN_DIR'." >&2
  exit 1
}
for t in clang-cl lld-link llvm-lib llvm-rc cmake ninja curl unzip git; do
  command -v "$t" >/dev/null || { echo "error: missing tool: $t" >&2; exit 1; }
done
mkdir -p "$SRC" "$WINCROSS_PREFIX"
echo ":: XWIN_DIR=$XWIN_DIR"
echo ":: prefix=$WINCROSS_PREFIX  (jobs=$JOBS)"

# fetch <url> <dir-glob> [sha256]  -> echoes the extracted source dir
fetch() {
  local url="$1" glob="$2" sha="${3:-}" f="$SRC/$(basename "$1")"
  [[ -f "$f" ]] || curl -fsSL -o "$f" "$url"
  [[ -z "$sha" ]] || echo "$sha  $f" | sha256sum -c - >/dev/null
  case "$f" in
    *.tar.gz|*.tgz) tar -C "$SRC" -xzf "$f" ;;
    *.zip)          (cd "$SRC" && unzip -qo "$f") ;;
  esac
  ( cd "$SRC" && ls -d $glob | head -1 )
}

# The toolchain + the two fixes, plus the OCCT-era CMake policy floor.
xcmake() {
  cmake -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$here/toolchain.cmake" \
    -DCMAKE_USER_MAKE_RULES_OVERRIDE="$here/override.cmake" \
    -DCMAKE_PROJECT_INCLUDE="$here/rc-sdk-includes.cmake" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$WINCROSS_PREFIX" \
    -DCMAKE_PREFIX_PATH="$WINCROSS_PREFIX" \
    -DBUILD_SHARED_LIBS=OFF "$@"
}
build() { ninja -C "$1" -j "$JOBS" install; }   # <build-dir>

# ── deps (dependency order) ──────────────────────────────────────────────

zlib() {  # validated
  local s; s="$SRC/$(fetch https://github.com/madler/zlib/archive/refs/tags/v1.3.1.zip 'zlib-*')"
  xcmake -S "$s" -B "$s/b"; build "$s/b"
}

tbb() {   # validated
  local s; s="$SRC/$(fetch https://github.com/oneapi-src/oneTBB/archive/refs/tags/v2021.5.0.zip 'oneTBB-*')"
  xcmake -S "$s" -B "$s/b" -DTBB_TEST=OFF -DTBB_STRICT=OFF; build "$s/b"
}

openexr() {  # validated — provides IlmBase/Half (OpenVDB's half type)
  local s; s="$SRC/$(fetch https://github.com/AcademySoftwareFoundation/openexr/archive/refs/tags/v2.5.5.zip 'openexr-*')"
  xcmake -S "$s" -B "$s/b" -DBUILD_TESTING=OFF -DOPENEXR_BUILD_UTILS=OFF \
    -DPYILMBASE_ENABLE=OFF -DOPENEXR_VIEWERS_ENABLE=OFF -DINSTALL_OPENEXR_EXAMPLES=OFF
  build "$s/b"
}

occt() {  # validated (C++ compiles clean; args mirror OrcaSlicer deps/OCCT)
  local s; s="$SRC/$(fetch https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/V7_6_0.zip 'OCCT-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix \
      "$here/../../external/OrcaSlicer/deps/OCCT/0001-OCCT-fix.patch" 2>/dev/null || true )
  xcmake -S "$s" -B "$s/b" -DCMAKE_CXX_STANDARD=17 -DBUILD_LIBRARY_TYPE=Static \
    -DUSE_TK=OFF -DUSE_TBB=OFF -DUSE_FREETYPE=OFF -DUSE_FFMPEG=OFF -DUSE_VTK=OFF \
    -DBUILD_DOC_Overview=OFF -DBUILD_MODULE_ApplicationFramework=OFF \
    -DBUILD_MODULE_Draw=OFF -DBUILD_MODULE_FoundationClasses=OFF \
    -DBUILD_MODULE_ModelingAlgorithms=OFF -DBUILD_MODULE_ModelingData=OFF \
    -DBUILD_MODULE_Visualization=OFF
  build "$s/b"
}

boost() {  # WIP — b2 cross via the clang-win toolset; libslic3r's component set
  local s; s="$SRC/$(fetch https://archives.boost.io/release/1.84.0/source/boost_1_84_0.tar.gz 'boost_1_84_0')"
  ( cd "$s" && ./bootstrap.sh
    cat > user-config.jam <<JAM
using clang-win : 14 : clang-cl :
  <compileflags>"--target=x86_64-pc-windows-msvc -fuse-ld=lld-link /imsvc$XWIN_DIR/crt/include /imsvc$XWIN_DIR/sdk/include/ucrt /imsvc$XWIN_DIR/sdk/include/um /imsvc$XWIN_DIR/sdk/include/shared"
  <linkflags>"-libpath:$XWIN_DIR/crt/lib/x86_64 -libpath:$XWIN_DIR/sdk/lib/um/x86_64 -libpath:$XWIN_DIR/sdk/lib/ucrt/x86_64"
  <archiver>llvm-lib <ranlib>llvm-lib ;
JAM
    ./b2 --user-config=user-config.jam --prefix="$WINCROSS_PREFIX" -j"$JOBS" \
      toolset=clang-win target-os=windows address-model=64 link=static \
      runtime-link=shared variant=release --layout=system \
      --with-system --with-filesystem --with-thread --with-iostreams \
      --with-log --with-locale --with-regex --with-nowide install )
  # NOTE: the clang-win/cross toolset config is unvalidated — expect iteration.
}

openvdb() {  # WIP — needs boost (iostreams+system); fork carries a clang patch
  local s; s="$SRC/$(fetch https://github.com/tamasmeszaros/openvdb/archive/a68fd58d0e2b85f01adeb8b13d7555183ab10aa5.zip 'openvdb-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix \
      "$here/../../external/OrcaSlicer/deps/OpenVDB/0001-clang19.patch" 2>/dev/null || true )
  xcmake -S "$s" -B "$s/b" -DOPENVDB_CORE_STATIC=ON -DOPENVDB_CORE_SHARED=OFF \
    -DOPENVDB_BUILD_PYTHON_MODULE=OFF -DOPENVDB_BUILD_VDB_PRINT=OFF \
    -DUSE_BLOSC=OFF -DUSE_EXR=OFF -DUSE_ZLIB=ON -DTBB_STATIC=ON \
    -DDISABLE_DEPENDENCY_VERSION_CHECKS=ON
  build "$s/b"
}

# Build in dependency order. Comment out / extend as deps are validated.
zlib
tbb
openexr
occt
boost      # WIP
openvdb    # WIP — depends on boost
# TODO: Blosc, CGAL, Cereal, Eigen, Qhull, NLopt, … (same `dep` pattern;
#       pins in external/OrcaSlicer/deps/<Name>/), then libslic3r itself.

echo ":: done. cross deps in $WINCROSS_PREFIX"
