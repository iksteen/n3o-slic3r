#!/usr/bin/env bash
# Cross-build OrcaSlicer's libslic3r dependency tree to x86_64-pc-windows-msvc
# from Linux — clang-cl + LLD + the MSVC CRT/SDK (cargo-xwin), driven through
# Ninja (OrcaSlicer's own deps-windows.cmake is VS-generator/msbuild-native and
# does NOT cross; this rebuilds each dep with the toolchain in this directory).
#
# STATUS (2026-06-07): the ENTIRE libslic3r dep tree cross-compiles clean —
# zlib, TBB, OpenEXR/IlmBase, OCCT, Boost, OpenVDB, Blosc, Cereal, Eigen,
# Qhull, NLopt, CGAL. GMP/MPFR don't cross (configure + asm) so we reuse
# OrcaSlicer's vendored MSVC prebuilts (already MSVC-ABI). Next is libslic3r
# itself + the FFI shim + the Tauri bundle — integration, not dep walls.
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
ORCA="$here/../../external/OrcaSlicer/deps"   # version pins + patches we reuse
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
      "$ORCA/OCCT/0001-OCCT-fix.patch" 2>/dev/null || true )
  xcmake -S "$s" -B "$s/b" -DCMAKE_CXX_STANDARD=17 -DBUILD_LIBRARY_TYPE=Static \
    -DUSE_TK=OFF -DUSE_TBB=OFF -DUSE_FREETYPE=OFF -DUSE_FFMPEG=OFF -DUSE_VTK=OFF \
    -DBUILD_DOC_Overview=OFF -DBUILD_MODULE_ApplicationFramework=OFF \
    -DBUILD_MODULE_Draw=OFF -DBUILD_MODULE_FoundationClasses=OFF \
    -DBUILD_MODULE_ModelingAlgorithms=OFF -DBUILD_MODULE_ModelingData=OFF \
    -DBUILD_MODULE_Visualization=OFF
  build "$s/b"
}

boost() {  # validated — Boost's own CMake build (NOT b2); clang-cl cross-clean
  local s; s="$SRC/$(fetch https://archives.boost.io/release/1.84.0/source/boost_1_84_0.tar.gz 'boost_1_84_0')"
  # The modular CMake build only *installs* headers for the selected compiled
  # libs (+ their deps). It cross-builds clean under clang-cl (the legacy-C
  # warning downgrades in toolchain.cmake are what unblock boost.container's
  # dlmalloc). libslic3r's compiled-lib set:
  # OrcaSlicer's find_package COMPONENT set (CMakeLists.txt). log pulls
  # log_setup; chrono/atomic/date_time/container come in transitively.
  xcmake -S "$s" -B "$s/b" -DBUILD_TESTING=OFF -DBOOST_RUNTIME_LINK=shared \
    -DBOOST_INCLUDE_LIBRARIES="system;filesystem;thread;log;locale;regex;iostreams;program_options;nowide"
  build "$s/b"
  # …but OpenVDB and libslic3r also pull header-only boost (any, interprocess,
  # …) that the selective install omits. Lay down the complete pre-assembled
  # header tree (what b2's `install` would have done) so every header resolves.
  cp -rn "$s/boost" "$WINCROSS_PREFIX/include/"
}

openvdb() {  # validated — links libopenvdb.lib; fork carries a clang19 patch
  local s; s="$SRC/$(fetch https://github.com/tamasmeszaros/openvdb/archive/a68fd58d0e2b85f01adeb8b13d7555183ab10aa5.zip 'openvdb-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix \
      "$ORCA/OpenVDB/0001-clang19.patch" 2>/dev/null || true )
  # USE_BLOSC=OFF for the minimal validated cross; flip ON once Blosc is built
  # (OrcaSlicer ships blosc-compressed VDB) — same toolchain, just another dep.
  xcmake -S "$s" -B "$s/b" -DOPENVDB_CORE_STATIC=ON -DOPENVDB_CORE_SHARED=OFF \
    -DOPENVDB_BUILD_PYTHON_MODULE=OFF -DOPENVDB_BUILD_VDB_PRINT=OFF \
    -DUSE_BLOSC=OFF -DUSE_EXR=OFF -DUSE_ZLIB=ON -DTBB_STATIC=ON \
    -DDISABLE_DEPENDENCY_VERSION_CHECKS=ON
  build "$s/b"
}

blosc() {  # validated — c-blosc (Orca's tm fork); uses our zlib (PREFER_EXTERNAL)
  local s; s="$SRC/$(fetch https://github.com/tamasmeszaros/c-blosc/archive/refs/heads/v1.17.0_tm.zip 'c-blosc-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix \
      "$ORCA/Blosc/blosc-mods.patch" 2>/dev/null || true )
  # blosc's CPack block does include(InstallRequiredSystemLibraries), whose MSVC
  # branch queries the Windows registry (can't cross). blosc points
  # CMAKE_MODULE_PATH at its own cmake/ dir, so shadow the builtin *there*.
  cp "$here/cmake-stubs/InstallRequiredSystemLibraries.cmake" "$s/cmake/"
  xcmake -S "$s" -B "$s/b" -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DBUILD_SHARED=OFF -DBUILD_STATIC=ON -DBUILD_TESTS=OFF -DBUILD_BENCHMARKS=OFF \
    -DPREFER_EXTERNAL_ZLIB=ON -DDEACTIVATE_SSE2=ON -DDEACTIVATE_AVX2=ON
  build "$s/b"
}

gmp_mpfr() {  # vendored MSVC prebuilts (x64) — GMP/MPFR don't cross (configure +
              # asm); OrcaSlicer ships prebuilt import libs+DLLs, which are
              # MSVC-ABI and link fine here. CGAL/libslic3r consume them.
  local g="$ORCA/GMP/gmp" m="$ORCA/MPFR/mpfr"
  mkdir -p "$WINCROSS_PREFIX"/{include,lib,bin}
  cp "$g/include/gmp.h" "$m/include/mpfr.h" "$m/include/mpf2mpfr.h" "$WINCROSS_PREFIX/include/"
  cp "$g/lib/win-x64/libgmp-10.lib"  "$WINCROSS_PREFIX/lib/"; cp "$g/lib/win-x64/libgmp-10.dll"  "$WINCROSS_PREFIX/bin/"
  cp "$m/lib/win-x64/libmpfr-4.lib"  "$WINCROSS_PREFIX/lib/"; cp "$m/lib/win-x64/libmpfr-4.dll"  "$WINCROSS_PREFIX/bin/"
  # also expose under the unversioned names CGAL's find_package looks for
  cp "$WINCROSS_PREFIX/lib/libgmp-10.lib" "$WINCROSS_PREFIX/lib/gmp.lib"
  cp "$WINCROSS_PREFIX/lib/libmpfr-4.lib" "$WINCROSS_PREFIX/lib/mpfr.lib"
}

cereal() {  # validated — header-only
  local s; s="$SRC/$(fetch https://github.com/USCiLab/cereal/archive/refs/tags/v1.3.0.zip 'cereal-*')"
  xcmake -S "$s" -B "$s/b" -DJUST_INSTALL_CEREAL=ON -DSKIP_PERFORMANCE_COMPARISON=ON -DBUILD_TESTS=OFF
  build "$s/b"
}

eigen() {  # validated — header-only
  local s; s="$SRC/$(fetch https://gitlab.com/libeigen/eigen/-/archive/5.0.1/eigen-5.0.1.zip 'eigen-*')"
  xcmake -S "$s" -B "$s/b" -DEIGEN_BUILD_DOC=OFF -DBUILD_TESTING=OFF -DEIGEN_BUILD_PKGCONFIG=OFF
  build "$s/b"
}

qhull() {  # validated
  local s; s="$SRC/$(fetch https://github.com/qhull/qhull/archive/v8.0.2.zip 'qhull-*')"
  xcmake -S "$s" -B "$s/b" -DINCLUDE_INSTALL_DIR=include
  build "$s/b"
}

nlopt() {  # validated
  local s; s="$SRC/$(fetch https://github.com/stevengj/nlopt/archive/v2.5.0.tar.gz 'nlopt-*')"
  xcmake -S "$s" -B "$s/b" -DNLOPT_PYTHON=OFF -DNLOPT_OCTAVE=OFF -DNLOPT_MATLAB=OFF \
    -DNLOPT_GUILE=OFF -DNLOPT_SWIG=OFF -DNLOPT_TESTS=OFF
  build "$s/b"
}

cgal() {  # validated — header-only; find_package needs GMP/MPFR/Boost in prefix
  local s; s="$SRC/$(fetch https://github.com/CGAL/cgal/releases/download/v5.6.3/CGAL-5.6.3.zip 'CGAL-*')"
  xcmake -S "$s" -B "$s/b" -DCGAL_HEADER_ONLY=ON -DWITH_examples=OFF -DWITH_demos=OFF -DWITH_CGAL_Qt5=OFF
  build "$s/b"
}

# ── libslic3r's image / font / misc deps ────────────────────────────────

png() {  # validated — lib cross-builds; its symlink *install* step can't cross,
         # so build the lib target and install by hand
  local s; s="$SRC/$(fetch https://github.com/glennrp/libpng/archive/refs/tags/v1.6.35.zip 'libpng-*')"
  xcmake -S "$s" -B "$s/b" -DPNG_SHARED=OFF -DPNG_TESTS=OFF -DPNG_EXECUTABLES=OFF -DZLIB_ROOT="$WINCROSS_PREFIX"
  ninja -C "$s/b" -j "$JOBS" png_static
  cp "$s/b/libpng16_static.lib" "$WINCROSS_PREFIX/lib/libpng16.lib"
  cp "$WINCROSS_PREFIX/lib/libpng16.lib" "$WINCROSS_PREFIX/lib/libpng.lib"
  cp "$s/png.h" "$s/pngconf.h" "$s/b/pnglibconf.h" "$WINCROSS_PREFIX/include/"
}

freetype() {  # validated — the © in its version .rc is handled by the cp1252
              # llvm-rc wrapper (toolchain.cmake), no source edit needed
  local s; s="$SRC/$(fetch https://github.com/SoftFever/orca_deps/releases/download/freetype-2.12.1.tar.gz/freetype-2.12.1.tar.gz 'freetype-*')"
  xcmake -S "$s" -B "$s/b" -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON
  build "$s/b"
}

glfw() {  # validated
  local s; s="$SRC/$(fetch https://github.com/glfw/glfw/archive/refs/tags/3.4.zip 'glfw-*')"
  xcmake -S "$s" -B "$s/b" -DGLFW_BUILD_EXAMPLES=OFF -DGLFW_BUILD_TESTS=OFF -DGLFW_BUILD_DOCS=OFF
  build "$s/b"
}

expat() {  # validated — compile OrcaSlicer's *bundled* expat so find_package(EXPAT)
           # succeeds; otherwise its not-found fallback double-defines the `expat`
           # target against deps_src/expat. No CMake build → drive clang-cl directly.
  local ed="$ORCA/../deps_src/expat" b="$BUILD_DIR/expat-obj"; mkdir -p "$b"
  for f in xmlparse xmlrole xmltok; do
    clang-cl --target=x86_64-pc-windows-msvc -Wno-unused-command-line-argument \
      /imsvc"$XWIN_DIR"/crt/include /imsvc"$XWIN_DIR"/sdk/include/ucrt \
      /imsvc"$XWIN_DIR"/sdk/include/um /imsvc"$XWIN_DIR"/sdk/include/shared \
      -DXML_STATIC -DHAVE_EXPAT_CONFIG_H -DWIN32 -I"$ed" /c "$ed/$f.c" /Fo"$b/$f.obj"
  done
  llvm-lib /out:"$WINCROSS_PREFIX/lib/libexpat.lib" "$b"/xmlparse.obj "$b"/xmlrole.obj "$b"/xmltok.obj
  cp "$ed/expat.h" "$ed/expat_external.h" "$WINCROSS_PREFIX/include/"
}

libnoise() {  # validated — Orca's libnoise fork
  local s; s="$SRC/$(fetch https://github.com/SoftFever/Orca-deps-libnoise/archive/refs/tags/1.0.zip 'Orca-deps-libnoise-*')"
  xcmake -S "$s" -B "$s/b"; build "$s/b"
}

jpeg() {  # validated — libjpeg-turbo; SIMD off avoids needing nasm
  local s; s="$SRC/$(fetch https://github.com/libjpeg-turbo/libjpeg-turbo/archive/refs/tags/3.0.1.zip 'libjpeg-turbo-*')"
  xcmake -S "$s" -B "$s/b" -DWITH_SIMD=OFF -DENABLE_SHARED=OFF -DENABLE_STATIC=ON
  build "$s/b"
  cp "$WINCROSS_PREFIX/lib/jpeg-static.lib" "$WINCROSS_PREFIX/lib/jpeg.lib"     # names FindJPEG looks for
  cp "$WINCROSS_PREFIX/lib/jpeg-static.lib" "$WINCROSS_PREFIX/lib/libjpeg.lib"
}

draco() {  # validated
  local s; s="$SRC/$(fetch https://github.com/google/draco/archive/refs/tags/1.5.7.zip 'draco-*')"
  xcmake -S "$s" -B "$s/b" -DDRACO_TESTS=OFF; build "$s/b"
}

opencv() {  # validated — OrcaSlicer's `world` build; libslic3r links opencv_world,
            # whose imported target also carries the include dir. core+imgproc give
            # cv::Mat/kmeans/cvtColor (ObjColorUtils); imgcodecs+highgui satisfy the
            # world module's wiring. quirc/ade off (not built → dangling exports).
  local s; s="$SRC/$(fetch https://github.com/opencv/opencv/archive/refs/tags/4.6.0.tar.gz 'opencv-*')"
  xcmake -S "$s" -B "$s/b" -DBUILD_LIST="core,imgcodecs,imgproc,highgui,world" -DBUILD_opencv_world=ON \
    -DBUILD_TESTS=OFF -DBUILD_PERF_TESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_opencv_apps=OFF -DBUILD_JAVA=OFF \
    -DBUILD_JPEG=ON -DBUILD_PNG=ON -DBUILD_ZLIB=OFF -DBUILD_OPENEXR=OFF \
    -DWITH_IPP=OFF -DWITH_ITT=OFF -DWITH_CUDA=OFF -DWITH_OPENCL=OFF -DWITH_EIGEN=OFF -DWITH_FFMPEG=OFF \
    -DWITH_GTK=OFF -DWITH_QT=OFF -DWITH_QUIRC=OFF -DWITH_ADE=OFF -DWITH_TIFF=OFF -DWITH_WEBP=OFF \
    -DWITH_OPENJPEG=OFF -DWITH_JASPER=OFF -DWITH_PROTOBUF=OFF -DWITH_1394=OFF -DWITH_DSHOW=OFF -DWITH_MSMF=OFF
  build "$s/b"
}

# OpenSSL + CURL — INTERIM (libslic3r *compile* only). libslic3r #include's just
# <openssl/md5.h> and, as a *static* archive, links neither; CURL it doesn't use
# at all. So real OpenSSL headers (generated via Configure) + CURL headers + empty
# stub import libs are enough to satisfy find_package and compile libslic3r.
# The FFI-shim DLL *link* needs real OpenSSL/CURL MSVC cross-builds — replace there.
openssl_curl_stub() {
  local o; o="$SRC/$(fetch https://github.com/openssl/openssl/archive/OpenSSL_1_1_1w.tar.gz 'openssl-OpenSSL_1_1_1w')"
  ( cd "$o" && ./Configure mingw64 no-asm no-shared --prefix=/tmp/none >/dev/null 2>&1 \
       && make include/openssl/opensslconf.h >/dev/null 2>&1 )
  mkdir -p "$WINCROSS_PREFIX/include/openssl"; cp "$o"/include/openssl/*.h "$WINCROSS_PREFIX/include/openssl/"
  local c; c="$SRC/$(fetch https://github.com/curl/curl/archive/refs/tags/curl-7_75_0.zip 'curl-curl-7_75_0')"
  mkdir -p "$WINCROSS_PREFIX/include/curl"; cp "$c"/include/curl/*.h "$WINCROSS_PREFIX/include/curl/"
  : > "$BUILD_DIR/empty.c"; clang-cl --target=x86_64-pc-windows-msvc /c /Fo"$BUILD_DIR/empty.obj" "$BUILD_DIR/empty.c"
  for l in libcrypto libssl crypto ssl libcurl; do llvm-lib /out:"$WINCROSS_PREFIX/lib/$l.lib" "$BUILD_DIR/empty.obj"; done
}

# Apply the clang-cl source-conformance patch to the OrcaSlicer submodule. The
# submodule tree stays pinned — this is in-place build prep; revert with
# `git -C external/OrcaSlicer checkout -- src/libslic3r/AABBTreeLines.hpp`.
patch_orca() {
  local p="$here/patches/0001-AABBTreeLines-eigen-cast-conformance.patch"
  if git -C "$ORCA/.." apply --check "$p" 2>/dev/null; then
    git -C "$ORCA/.." apply "$p"; echo ":: applied $(basename "$p")"
  else
    echo ":: $(basename "$p") already applied (or no longer needed)"
  fi
}

# ── Build in dependency order ────────────────────────────────────────────
# Geometry / math
zlib; tbb; openexr; occt; boost; openvdb; blosc; gmp_mpfr; cereal; eigen; qhull; nlopt; cgal
# Image / font / misc (libslic3r)
png; freetype; glfw; expat; libnoise; jpeg; draco; opencv
openssl_curl_stub
patch_orca
# With these + the source patch, libslic3r cross-compiles to a windows-msvc
# COFF archive (validated: 255/255 objects -> libslic3r.lib). Next: the FFI
# shim DLL (needs real OpenSSL/CURL) + src-tauri (cargo-xwin) + the NSIS bundle.

echo ":: done. cross deps in $WINCROSS_PREFIX"
