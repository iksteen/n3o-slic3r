cmake_minimum_required(VERSION 3.13)
project(orca-slicer-ffi LANGUAGES C CXX)

# Drives an out-of-tree build of OrcaSlicer's libslic3r plus a thin C FFI
# layer (libslic3r_ffi.so). OrcaSlicer itself lives in external/OrcaSlicer
# as an unmodified git submodule; the shim is built against it.

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# OrcaSlicer's deps tree (Boost/CGAL/OCCT/TBB/...) is built separately via
# external/OrcaSlicer/build_linux.sh -d. Default the prefix to its standard
# location; allow override for shared / packaged builds.
if(NOT DEFINED CACHE{CMAKE_PREFIX_PATH} OR "${CMAKE_PREFIX_PATH}" STREQUAL "")
    set(CMAKE_PREFIX_PATH
        "${CMAKE_SOURCE_DIR}/external/OrcaSlicer/deps/build/OrcaSlicer_dep/usr/local"
        CACHE PATH "OrcaSlicer deps prefix" FORCE)
endif()
message(STATUS "FFI: CMAKE_PREFIX_PATH = ${CMAKE_PREFIX_PATH}")

# OrcaSlicer's bundled boost was built with CMake's older FindBoost behavior.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)

# Suppress everything in upstream Orca we don't want built. SLIC3R_GUI off
# avoids wxWidgets / OpenGL / the entire src/slic3r tree. The rest just
# silence optional subtrees.
set(SLIC3R_GUI             OFF CACHE BOOL "" FORCE)
set(SLIC3R_BUILD_SANDBOXES OFF CACHE BOOL "" FORCE)
set(BUILD_TESTS            OFF CACHE BOOL "" FORCE)
set(ORCA_TOOLS             OFF CACHE BOOL "" FORCE)
# Upstream defines this; flipping it on keeps the build artifact tree we hook into.
set(ORCA_BUILD_FFI         OFF CACHE BOOL "" FORCE)

# Pull in OrcaSlicer's tree with EXCLUDE_FROM_ALL: every target (libslic3r,
# the OrcaSlicer executable, etc.) is registered but not built unless
# something depends on it. Our slic3r_ffi target depends on libslic3r — so
# libslic3r and its transitive static deps build; the OrcaSlicer executable
# does not.
add_subdirectory(external/OrcaSlicer EXCLUDE_FROM_ALL)

# The shim. Linking it pulls in libslic3r and friends.
add_subdirectory(ffi)
