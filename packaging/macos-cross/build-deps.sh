#!/usr/bin/env bash
# Cross-build OrcaSlicer's libslic3r dependency tree to *-apple-darwin from
# Linux using osxcross (clang + cctools/ld64 + a packaged macOS SDK). Mirrors
# packaging/windows-cross/build-deps.sh: OrcaSlicer's own deps-macos.cmake
# superbuild does NOT cross (it bootstraps a host `b2` for Boost and assumes a
# native Apple toolchain), so this rebuilds each dep with the osxcross toolchain
# instead, into the SAME arch-namespaced prefix the native mac build uses
#   external/OrcaSlicer/deps/build/<arch>/OrcaSlicer_dep/usr/local
# so crates/slic3r-ffi/build.rs (its macOS branch) finds it unchanged.
#
# Prereqs:
#   - osxcross built at ~/osxcross (OSXCROSS_TARGET_DIR/bin on demand below),
#     with a MacOSX SDK packaged in. `osxcross-conf` must be on PATH or under
#     $OSXCROSS_ROOT/bin.
#   - Arch host pkgs: clang lld llvm cmake ninja curl unzip git
#
# Usage:   ./build-deps.sh [arm64|x86_64]      (default: arm64)
#
# Env:
#   OSXCROSS_ROOT   osxcross install dir.  Default: $HOME/osxcross/target
#   BUILD_DIR       scratch (sources + per-dep builds). Default: ./.build
#   MACCROSS_PREFIX install prefix. Default: the arch-namespaced OrcaSlicer path
#   JOBS            parallelism. Default: nproc
#
# Per-dep stamps under $MACCROSS_PREFIX/.stamps let a re-run resume after a
# fixed failure without rebuilding the whole tree. Delete a stamp (or the
# prefix) to force a rebuild.
set -euo pipefail

ARCH="${1:-arm64}"
case "$ARCH" in arm64|x86_64) ;; *) echo "arch must be arm64 or x86_64" >&2; exit 2;; esac

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$here/../.." && pwd)"
ORCA="$REPO_ROOT/external/OrcaSlicer/deps"   # version pins + patches we reuse

: "${OSXCROSS_ROOT:=$HOME/osxcross/target}"
export PATH="$OSXCROSS_ROOT/bin:$PATH"
command -v osxcross-conf >/dev/null || { echo "error: osxcross-conf not on PATH (OSXCROSS_ROOT=$OSXCROSS_ROOT)" >&2; exit 1; }
eval "$(osxcross-conf)"                       # OSXCROSS_TARGET_DIR, OSXCROSS_SDK, OSXCROSS_TARGET, OSXCROSS_SDK_VERSION
export OSXCROSS_TARGET_DIR OSXCROSS_SDK OSXCROSS_TARGET
DARWIN="${OSXCROSS_TARGET#darwin}"            # e.g. 24.4
export OSXCROSS_HOST="${ARCH}-apple-darwin${DARWIN}"
# Automake/configure projects (GMP/MPFR) want the aarch64 spelling, not arm64.
HOST_TRIPLE="$([ "$ARCH" = arm64 ] && echo "aarch64-apple-darwin${DARWIN}" || echo "x86_64-apple-darwin${DARWIN}")"
CC_WRAP="$OSXCROSS_TARGET_DIR/bin/${HOST_TRIPLE}-clang"
CXX_WRAP="$OSXCROSS_TARGET_DIR/bin/${HOST_TRIPLE}-clang++"

: "${BUILD_DIR:=$here/.build}"
: "${MACCROSS_PREFIX:=$REPO_ROOT/external/OrcaSlicer/deps/build/${ARCH}/OrcaSlicer_dep/usr/local}"
: "${JOBS:=$(nproc)}"
export MACCROSS_PREFIX
SRC="$BUILD_DIR/$ARCH/src"
STAMPS="$MACCROSS_PREFIX/.stamps"
DEPLOY=11.3
TC="$here/toolchain.cmake"

for t in clang cmake ninja curl unzip git perl make; do
  command -v "$t" >/dev/null || { echo "error: missing tool: $t" >&2; exit 1; }
done
[ -x "$CC_WRAP" ] || { echo "error: osxcross wrapper not found: $CC_WRAP" >&2; exit 1; }
mkdir -p "$SRC" "$MACCROSS_PREFIX/lib" "$MACCROSS_PREFIX/include" "$STAMPS"

echo ":: arch=$ARCH host=$OSXCROSS_HOST SDK=$OSXCROSS_SDK_VERSION deploy=$DEPLOY"
echo ":: prefix=$MACCROSS_PREFIX (jobs=$JOBS)"

# stamped <name> <fn>  — run dep builder once; skip if its stamp exists.
stamped() {
  local name="$1"; shift
  if [ -f "$STAMPS/$name" ]; then echo ":: [$name] cached, skipping"; return 0; fi
  echo ":: [$name] building…"
  "$@"
  touch "$STAMPS/$name"
  echo ":: [$name] done"
}

# fetch <url> <dir-glob> [sha256] -> echoes the extracted source dir name
fetch() {
  local url="$1" glob="$2" sha="${3:-}" f="$SRC/$(basename "$1")"
  [[ -f "$f" ]] || curl -fsSL -o "$f" "$url"
  [[ -z "$sha" ]] || echo "$sha  $f" | sha256sum -c - >/dev/null
  case "$f" in
    *.tar.gz|*.tgz) tar -C "$SRC" -xzf "$f" ;;
    *.tar.bz2)      tar -C "$SRC" -xjf "$f" ;;
    *.tar.xz)       tar -C "$SRC" -xJf "$f" ;;
    *.zip)          (cd "$SRC" && unzip -qo "$f") ;;
  esac
  ( cd "$SRC" && ls -d $glob | head -1 )
}

# xcmake — configure through the osxcross toolchain, static, into the prefix.
xcmake() {
  cmake -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$TC" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DCMAKE_OSX_ARCHITECTURES="$ARCH" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOY" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$MACCROSS_PREFIX" \
    -DCMAKE_PREFIX_PATH="$MACCROSS_PREFIX" \
    -DBUILD_SHARED_LIBS=OFF "$@"
}
build() { ninja -C "$1" -j "$JOBS" install; }   # <build-dir>

# ── deps (dependency order) ──────────────────────────────────────────────

zlib() {  # static only — install libz.a by hand (zlib always builds both)
  local s; s="$SRC/$(fetch https://github.com/madler/zlib/archive/refs/tags/v1.3.1.zip 'zlib-*')"
  xcmake -S "$s" -B "$s/b"
  ninja -C "$s/b" -j "$JOBS" zlibstatic
  cp "$s/b/libz.a" "$MACCROSS_PREFIX/lib/libz.a"
  cp "$s/zlib.h" "$s/b/zconf.h" "$MACCROSS_PREFIX/include/"
}

tbb() {
  local s; s="$SRC/$(fetch https://github.com/oneapi-src/oneTBB/archive/refs/tags/v2021.5.0.zip 'oneTBB-*')"
  xcmake -S "$s" -B "$s/b" -DTBB_TEST=OFF -DTBB_STRICT=OFF; build "$s/b"
}

openexr() {  # provides IlmBase/Half (OpenVDB's half type)
  local s; s="$SRC/$(fetch https://github.com/AcademySoftwareFoundation/openexr/archive/refs/tags/v2.5.5.zip 'openexr-*')"
  xcmake -S "$s" -B "$s/b" -DBUILD_TESTING=OFF -DOPENEXR_BUILD_UTILS=OFF \
    -DPYILMBASE_ENABLE=OFF -DOPENEXR_VIEWERS_ENABLE=OFF -DINSTALL_OPENEXR_EXAMPLES=OFF
  build "$s/b"
}

freetype() {
  local s; s="$SRC/$(fetch https://github.com/SoftFever/orca_deps/releases/download/freetype-2.12.1.tar.gz/freetype-2.12.1.tar.gz 'freetype-*')"
  xcmake -S "$s" -B "$s/b" -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON
  build "$s/b"
}

occt() {  # must build WITH freetype (TKService references FT_* unguarded); reuse Orca's patch
  local s; s="$SRC/$(fetch https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/V7_6_0.zip 'OCCT-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix "$ORCA/OCCT/0001-OCCT-fix.patch" 2>/dev/null || true )
  local ftinc; ftinc="$(dirname "$(find "$MACCROSS_PREFIX/include" -name ft2build.h -print -quit)")"
  [[ -n "$ftinc" && -f "$ftinc/ft2build.h" ]] || { echo "error: freetype headers missing (build freetype before occt)" >&2; return 1; }
  xcmake -S "$s" -B "$s/b" -DCMAKE_CXX_STANDARD=17 -DBUILD_LIBRARY_TYPE=Static \
    -DUSE_TK=OFF -DUSE_TBB=OFF -DUSE_FFMPEG=OFF -DUSE_VTK=OFF \
    -DUSE_FREETYPE=ON \
    -D3RDPARTY_FREETYPE_DIR="$MACCROSS_PREFIX" \
    -D3RDPARTY_FREETYPE_INCLUDE_DIR_ft2build="$ftinc" \
    -D3RDPARTY_FREETYPE_INCLUDE_DIR_freetype2="$ftinc" \
    -DBUILD_DOC_Overview=OFF -DBUILD_MODULE_ApplicationFramework=OFF \
    -DBUILD_MODULE_Draw=OFF -DBUILD_MODULE_FoundationClasses=OFF \
    -DBUILD_MODULE_ModelingAlgorithms=OFF -DBUILD_MODULE_ModelingData=OFF \
    -DBUILD_MODULE_Visualization=OFF
  build "$s/b"
}

boost() {  # Boost's own CMake build (NOT b2); the dashed superproject ships CMakeLists + writes BoostConfig.cmake
  local s; s="$SRC/$(fetch https://github.com/boostorg/boost/releases/download/boost-1.84.0/boost-1.84.0.tar.gz 'boost-1.84.0')"
  # Compiled-lib set = OrcaSlicer's find_package COMPONENTS. On darwin Boost.Context
  # uses its combined-syntax .S asm which clang's integrated assembler builds — no
  # ml64/gas override needed (unlike the Windows/PE target).
  # Boost.Iostreams auto-detects optional bzip2/lzma/zstd filters. With the
  # toolchain's BOTH find-mode those resolve to the HOST Linux libs, but the
  # cross compile uses the macOS sysroot (no lzma.h/zstd.h) and fails. libslic3r
  # only needs the zlib/gzip filter (zlib is in our prefix), so keep ZLIB and
  # turn the rest off.
  # Same host-leak guard for Boost.Locale: its ICU backend resolves to the host
  # Linux ICU but the macOS sysroot has no unicode/*.h. The iconv backend (iconv
  # is in the SDK) covers libslic3r's Locale use, so disable ICU.
  xcmake -S "$s" -B "$s/b" -DBUILD_TESTING=OFF \
    -DBOOST_IOSTREAMS_ENABLE_BZIP2=OFF -DBOOST_IOSTREAMS_ENABLE_LZMA=OFF -DBOOST_IOSTREAMS_ENABLE_ZSTD=OFF \
    -DBOOST_LOCALE_ENABLE_ICU=OFF \
    -DBOOST_INCLUDE_LIBRARIES="system;filesystem;thread;log;log_setup;locale;regex;chrono;atomic;date_time;iostreams;program_options;nowide"
  build "$s/b"
  # Assemble the full header set (header-only boost the selective install omits).
  mkdir -p "$MACCROSS_PREFIX/include/boost"
  find "$s/libs" -type d -path '*/include/boost' | sort | while IFS= read -r d; do
    cp -rn "$d/." "$MACCROSS_PREFIX/include/boost/"
  done
}

blosc() {  # c-blosc (Orca's tm fork); uses our zlib
  local s; s="$SRC/$(fetch https://github.com/tamasmeszaros/c-blosc/archive/refs/heads/v1.17.0_tm.zip 'c-blosc-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix "$ORCA/Blosc/blosc-mods.patch" 2>/dev/null || true )
  xcmake -S "$s" -B "$s/b" -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
    -DBUILD_SHARED=OFF -DBUILD_STATIC=ON -DBUILD_TESTS=OFF -DBUILD_BENCHMARKS=OFF \
    -DPREFER_EXTERNAL_ZLIB=ON -DDEACTIVATE_AVX2=ON
  build "$s/b"
}

openvdb() {  # links libopenvdb.a; fork carries a clang19 patch. USE_BLOSC=OFF matches the validated Windows config.
  local s; s="$SRC/$(fetch https://github.com/tamasmeszaros/openvdb/archive/a68fd58d0e2b85f01adeb8b13d7555183ab10aa5.zip 'openvdb-*')"
  ( cd "$s" && git apply --ignore-space-change --whitespace=fix "$ORCA/OpenVDB/0001-clang19.patch" 2>/dev/null || true )
  xcmake -S "$s" -B "$s/b" -DOPENVDB_CORE_STATIC=ON -DOPENVDB_CORE_SHARED=OFF \
    -DOPENVDB_BUILD_PYTHON_MODULE=OFF -DOPENVDB_BUILD_VDB_PRINT=OFF \
    -DUSE_BLOSC=OFF -DUSE_EXR=OFF -DUSE_ZLIB=ON -DTBB_STATIC=ON \
    -DDISABLE_DEPENDENCY_VERSION_CHECKS=ON
  build "$s/b"
}

gmp() {  # GMP cross-builds with autotools (unlike Windows, where it needed prebuilts)
  local s; s="$SRC/$(fetch https://gmplib.org/download/gmp/gmp-6.3.0.tar.xz 'gmp-6.3.0')"
  ( cd "$s" && [ -f Makefile ] || CC="$CC_WRAP" CXX="$CXX_WRAP" \
      CFLAGS="-mmacosx-version-min=$DEPLOY" \
      ./configure --host="$HOST_TRIPLE" --prefix="$MACCROSS_PREFIX" \
        --disable-shared --enable-static --enable-cxx >/dev/null )
  make -C "$s" -j "$JOBS"; make -C "$s" install
}

mpfr() {  # depends on gmp in the prefix
  local s; s="$SRC/$(fetch https://www.mpfr.org/mpfr-4.2.1/mpfr-4.2.1.tar.xz 'mpfr-4.2.1')"
  ( cd "$s" && [ -f Makefile ] || CC="$CC_WRAP" \
      CFLAGS="-mmacosx-version-min=$DEPLOY" \
      ./configure --host="$HOST_TRIPLE" --prefix="$MACCROSS_PREFIX" \
        --with-gmp="$MACCROSS_PREFIX" --disable-shared --enable-static >/dev/null )
  make -C "$s" -j "$JOBS"; make -C "$s" install
}

cereal() {  # header-only
  local s; s="$SRC/$(fetch https://github.com/USCiLab/cereal/archive/refs/tags/v1.3.0.zip 'cereal-*')"
  xcmake -S "$s" -B "$s/b" -DJUST_INSTALL_CEREAL=ON -DSKIP_PERFORMANCE_COMPARISON=ON -DBUILD_TESTS=OFF
  build "$s/b"
}

eigen() {  # header-only
  local s; s="$SRC/$(fetch https://gitlab.com/libeigen/eigen/-/archive/5.0.1/eigen-5.0.1.zip 'eigen-*')"
  # Eigen is header-only here; its optional BLAS/LAPACK shim libs default ON when
  # Eigen is the top-level project and their dylib link pulls compiler-rt complex
  # builtins (___divdc3) that the cross link doesn't resolve. We never link them.
  xcmake -S "$s" -B "$s/b" -DEIGEN_BUILD_DOC=OFF -DBUILD_TESTING=OFF -DEIGEN_BUILD_PKGCONFIG=OFF \
    -DEIGEN_BUILD_BLAS=OFF -DEIGEN_BUILD_LAPACK=OFF
  build "$s/b"
}

qhull() {
  local s; s="$SRC/$(fetch https://github.com/qhull/qhull/archive/v8.0.2.zip 'qhull-*')"
  xcmake -S "$s" -B "$s/b" -DINCLUDE_INSTALL_DIR=include
  build "$s/b"
}

nlopt() {
  local s; s="$SRC/$(fetch https://github.com/stevengj/nlopt/archive/v2.5.0.tar.gz 'nlopt-*')"
  xcmake -S "$s" -B "$s/b" -DNLOPT_PYTHON=OFF -DNLOPT_OCTAVE=OFF -DNLOPT_MATLAB=OFF \
    -DNLOPT_GUILE=OFF -DNLOPT_SWIG=OFF -DNLOPT_TESTS=OFF
  build "$s/b"
}

cgal() {  # header-only; find_package needs GMP/MPFR/Boost in prefix
  local s; s="$SRC/$(fetch https://github.com/CGAL/cgal/releases/download/v5.6.3/CGAL-5.6.3.zip 'CGAL-*')"
  xcmake -S "$s" -B "$s/b" -DCGAL_HEADER_ONLY=ON -DWITH_examples=OFF -DWITH_demos=OFF -DWITH_CGAL_Qt5=OFF
  build "$s/b"
}

# ── libslic3r's image / font / misc deps ────────────────────────────────

png() {  # build the static target; install by hand for a clean libpng.a
  local s; s="$SRC/$(fetch https://github.com/glennrp/libpng/archive/refs/tags/v1.6.35.zip 'libpng-*')"
  # pngpriv.h's Mac branch (#if TARGET_OS_MAC) pulls the long-gone Classic-Mac
  # <fp.h> unless <math.h>'s guard (__MATH_H__) is already defined. Force-include
  # <math.h> at the top of every TU: it defines its own guard (skipping fp.h) and
  # provides the floor/pow/frexp declarations the mac branch otherwise omits. (Do
  # NOT also -D__MATH_H__ — that is math.h's guard, so defining it blanks math.h.)
  # PNG_ARM_NEON=off: arm64's run-time NEON-check path (arm/arm_init.c) needs an
  # OS detection file osxcross has none of; NEON perf is irrelevant to this build.
  xcmake -S "$s" -B "$s/b" -DPNG_SHARED=OFF -DPNG_TESTS=OFF -DPNG_EXECUTABLES=OFF -DZLIB_ROOT="$MACCROSS_PREFIX" \
    -DPNG_ARM_NEON=off -DCMAKE_C_FLAGS="-include math.h"
  ninja -C "$s/b" -j "$JOBS" png_static
  cp "$s/b/libpng16.a" "$MACCROSS_PREFIX/lib/libpng16.a"
  cp "$MACCROSS_PREFIX/lib/libpng16.a" "$MACCROSS_PREFIX/lib/libpng.a"
  cp "$s/png.h" "$s/pngconf.h" "$s/b/pnglibconf.h" "$MACCROSS_PREFIX/include/"
}

glfw() {  # Cocoa backend (libslic3r/OCCT may reference it via headless GL context paths)
  local s; s="$SRC/$(fetch https://github.com/glfw/glfw/archive/refs/tags/3.4.zip 'glfw-*')"
  xcmake -S "$s" -B "$s/b" -DGLFW_BUILD_EXAMPLES=OFF -DGLFW_BUILD_TESTS=OFF -DGLFW_BUILD_DOCS=OFF
  build "$s/b"
}

expat() {  # OrcaSlicer's bundled expat has no install target — compile the 3 sources
           # directly into libexpat.a (mirrors the windows-cross build) so
           # find_package(EXPAT) resolves into the prefix.
  local ed="$ORCA/../deps_src/expat" b="$BUILD_DIR/$ARCH/expat-obj"; mkdir -p "$b"
  for f in xmlparse xmlrole xmltok; do
    "$CC_WRAP" -mmacosx-version-min="$DEPLOY" -O2 -DXML_STATIC -DHAVE_EXPAT_CONFIG_H \
      -I"$ed" -c "$ed/$f.c" -o "$b/$f.o"
  done
  "$OSXCROSS_TARGET_DIR/bin/${OSXCROSS_HOST}-ar" rcs "$MACCROSS_PREFIX/lib/libexpat.a" \
    "$b"/xmlparse.o "$b"/xmlrole.o "$b"/xmltok.o
  cp "$ed/expat.h" "$ed/expat_external.h" "$MACCROSS_PREFIX/include/"
}

libnoise() {  # Orca's libnoise fork
  local s; s="$SRC/$(fetch https://github.com/SoftFever/Orca-deps-libnoise/archive/refs/tags/1.0.zip 'Orca-deps-libnoise-*')"
  xcmake -S "$s" -B "$s/b"; build "$s/b"
}

jpeg() {  # libjpeg-turbo; SIMD off avoids nasm
  local s; s="$SRC/$(fetch https://github.com/libjpeg-turbo/libjpeg-turbo/archive/refs/tags/3.0.1.zip 'libjpeg-turbo-*')"
  xcmake -S "$s" -B "$s/b" -DWITH_SIMD=OFF -DENABLE_SHARED=OFF -DENABLE_STATIC=ON
  build "$s/b"
}

draco() {
  local s; s="$SRC/$(fetch https://github.com/google/draco/archive/refs/tags/1.5.7.zip 'draco-*')"
  # draco wraps each executable's libs in GNU --start-group/--end-group whenever
  # the compiler id matches ^Clang|^GNU. A native mac's id is "AppleClang" (no
  # match, so it's skipped); osxcross clang reports plain "Clang", so the group
  # flags fire and ld64 rejects --start-group. Skip them on Apple, as native does.
  sed -i 's/CMAKE_CXX_COMPILER_ID MATCHES "\^Clang|\^GNU"/CMAKE_CXX_COMPILER_ID MATCHES "^Clang|^GNU" AND NOT APPLE/' \
    "$s/cmake/draco_targets.cmake"
  xcmake -S "$s" -B "$s/b" -DDRACO_TESTS=OFF; build "$s/b"
}

opencv() {  # OrcaSlicer's `world` build; libslic3r links opencv_world (ObjColorUtils)
  local s; s="$SRC/$(fetch https://github.com/opencv/opencv/archive/refs/tags/4.6.0.tar.gz 'opencv-*')"
  # Use our prefix libpng/libjpeg/zlib rather than opencv's bundled copies:
  # bundled 3rdparty/libpng re-hits the macOS fp.h trap (and arm64 NEON). Our
  # prefix png is already patched for both. WITH_LAPACK=OFF avoids the host
  # CBLAS/LAPACK header probe (not in the SDK). libslic3r only uses opencv core/
  # imgproc (cvtColor/kmeans/Mat), so a thinner imgcodecs is fine.
  xcmake -S "$s" -B "$s/b" -DBUILD_LIST="core,imgcodecs,imgproc,highgui,world" -DBUILD_opencv_world=ON \
    -DBUILD_TESTS=OFF -DBUILD_PERF_TESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_opencv_apps=OFF -DBUILD_JAVA=OFF \
    -DBUILD_JPEG=OFF -DBUILD_PNG=OFF -DBUILD_ZLIB=OFF -DBUILD_OPENEXR=OFF -DWITH_OPENEXR=OFF \
    -DWITH_IPP=OFF -DWITH_ITT=OFF -DWITH_CUDA=OFF -DWITH_OPENCL=OFF -DWITH_EIGEN=OFF -DWITH_FFMPEG=OFF \
    -DWITH_GTK=OFF -DWITH_QT=OFF -DWITH_QUIRC=OFF -DWITH_ADE=OFF -DWITH_TIFF=OFF -DWITH_WEBP=OFF \
    -DWITH_OPENJPEG=OFF -DWITH_JASPER=OFF -DWITH_PROTOBUF=OFF -DWITH_1394=OFF \
    -DWITH_LAPACK=OFF -DWITH_GSTREAMER=OFF
  build "$s/b"
}

# OpenSSL + CURL. libslic3r's only OpenSSL use is MD5 (<openssl/md5.h>); compile
# OpenSSL's own crypto/md5/*.c (+ the OPENSSL_cleanse it calls) into a real
# libcrypto.a — byte-identical MD5, no full cross. libssl + CURL are never
# referenced by libslic3r, so those stay headers + empty stub libs.
openssl_curl() {
  local o; o="$SRC/$(fetch https://github.com/openssl/openssl/archive/OpenSSL_1_1_1w.tar.gz 'openssl-OpenSSL_1_1_1w')"
  local cfg; cfg="$([ "$ARCH" = arm64 ] && echo darwin64-arm64-cc || echo darwin64-x86_64-cc)"
  ( cd "$o" && [ -f include/openssl/opensslconf.h ] || {
      ./Configure "$cfg" no-asm no-shared --prefix=/tmp/none >/dev/null 2>&1
      make include/openssl/opensslconf.h >/dev/null 2>&1; } )
  mkdir -p "$MACCROSS_PREFIX/include/openssl"; cp "$o"/include/openssl/*.h "$MACCROSS_PREFIX/include/openssl/"
  local b="$BUILD_DIR/$ARCH/openssl-obj"; mkdir -p "$b"
  for f in crypto/md5/md5_dgst crypto/md5/md5_one crypto/mem_clr; do
    "$CC_WRAP" -mmacosx-version-min="$DEPLOY" -O2 -DOPENSSL_NO_ASM \
      -I"$o/include" -I"$o" -I"$o/crypto" -c "$o/$f.c" -o "$b/$(basename "$f").o"
  done
  "$OSXCROSS_TARGET_DIR/bin/${OSXCROSS_HOST}-ar" rcs "$MACCROSS_PREFIX/lib/libcrypto.a" \
    "$b"/md5_dgst.o "$b"/md5_one.o "$b"/mem_clr.o
  local c; c="$SRC/$(fetch https://github.com/curl/curl/archive/refs/tags/curl-7_75_0.zip 'curl-curl-7_75_0')"
  mkdir -p "$MACCROSS_PREFIX/include/curl"; cp "$c"/include/curl/*.h "$MACCROSS_PREFIX/include/curl/"
  : > "$b/empty.c"; "$CC_WRAP" -mmacosx-version-min="$DEPLOY" -c "$b/empty.c" -o "$b/empty.o"
  for l in libssl libcurl; do
    "$OSXCROSS_TARGET_DIR/bin/${OSXCROSS_HOST}-ar" rcs "$MACCROSS_PREFIX/lib/$l.a" "$b/empty.o"
  done
}

# Apply the same in-place OrcaSlicer source patches the native build relies on,
# if any are needed for cross. (None known yet — the engine + shim cross-build
# step in build.rs will surface them; add here as they appear.)
patch_orca() { :; }

# ── Build in dependency order ────────────────────────────────────────────
# Geometry / math. freetype before occt (TKService needs FT headers); gmp before
# mpfr before cgal.
stamped zlib zlib
stamped tbb tbb
stamped openexr openexr
stamped freetype freetype
stamped occt occt
stamped boost boost
stamped blosc blosc
stamped openvdb openvdb
stamped gmp gmp
stamped mpfr mpfr
stamped cereal cereal
stamped eigen eigen
stamped qhull qhull
stamped nlopt nlopt
stamped cgal cgal
# Image / font / misc (libslic3r)
stamped png png
stamped glfw glfw
stamped expat expat
stamped libnoise libnoise
stamped jpeg jpeg
stamped draco draco
stamped opencv opencv
stamped openssl_curl openssl_curl
patch_orca

: > "$MACCROSS_PREFIX/.deps-complete"
echo ":: done. cross deps for $ARCH in $MACCROSS_PREFIX"
