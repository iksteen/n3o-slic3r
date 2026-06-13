# macOS cross toolchain — thin wrapper over osxcross's generated toolchain.
#
# osxcross's own target/toolchain.cmake selects the compiler/sysroot from the
# OSXCROSS_* env (OSXCROSS_HOST picks the arch: arm64-/x86_64-apple-darwinNN).
# We include it verbatim, then make two adjustments the OrcaSlicer dep tree
# needs:
#
#   1. Add the cross-deps install prefix (MACCROSS_PREFIX) to the find-root
#      path so each dep discovers the ones built before it (OCCT finds the
#      freetype we just built, OpenVDB finds TBB, CGAL finds GMP/MPFR, …).
#   2. Relax the package/include/library find modes to BOTH. osxcross pins
#      them to ONLY (search the SDK sysroot only); the deps' own config
#      packages and headers live under MACCROSS_PREFIX, which is not the
#      sysroot, so ONLY hides them. BOTH still re-roots system finds into the
#      SDK — host Linux libraries are a different Mach-O-incompatible object
#      file and never satisfy a link even if a stray path is found.
#
# Used by packaging/macos-cross/build-deps.sh (the dep tree) and by
# crates/slic3r-ffi/build.rs (the libslic3r + shim cross-build).

if(NOT DEFINED ENV{OSXCROSS_TARGET_DIR})
    message(FATAL_ERROR "OSXCROSS_TARGET_DIR not set — source osxcross-conf first")
endif()

include("$ENV{OSXCROSS_TARGET_DIR}/toolchain.cmake")

if(DEFINED ENV{MACCROSS_PREFIX})
    list(APPEND CMAKE_FIND_ROOT_PATH "$ENV{MACCROSS_PREFIX}")
endif()

set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE BOTH)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE BOTH)
