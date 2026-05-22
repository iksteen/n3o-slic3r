cmake_minimum_required(VERSION 3.13)
project(slic3r_ffi LANGUAGES C CXX)

# Drives the cmake build of libslic3r and the FFI shim. Invoked from this
# crate's build.rs so `cargo build` produces a ready-to-link .so.
#
# Layout: this CMakeLists.txt lives in crates/slic3r-ffi/. OrcaSlicer is the
# workspace-level submodule at ../../external/OrcaSlicer/.

get_filename_component(WORKSPACE_ROOT "${CMAKE_SOURCE_DIR}/../.." ABSOLUTE)
set(ORCA_DIR "${WORKSPACE_ROOT}/external/OrcaSlicer")

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# OrcaSlicer's pre-built dep tree (Boost/CGAL/OCCT/TBB/...). The workspace's
# scripts/build.sh deps target produces this once; we assume it exists here.
if(NOT DEFINED CACHE{CMAKE_PREFIX_PATH} OR "${CMAKE_PREFIX_PATH}" STREQUAL "")
    set(CMAKE_PREFIX_PATH
        "${ORCA_DIR}/deps/build/OrcaSlicer_dep/usr/local"
        CACHE PATH "OrcaSlicer deps prefix" FORCE)
endif()
message(STATUS "slic3r-ffi: CMAKE_PREFIX_PATH = ${CMAKE_PREFIX_PATH}")

# OrcaSlicer's bundled boost was built with CMake's older FindBoost behavior.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)

# Suppress everything upstream Orca builds beyond libslic3r.
set(SLIC3R_GUI             OFF CACHE BOOL "" FORCE)
set(SLIC3R_BUILD_SANDBOXES OFF CACHE BOOL "" FORCE)
set(BUILD_TESTS            OFF CACHE BOOL "" FORCE)
set(ORCA_TOOLS             OFF CACHE BOOL "" FORCE)
set(ORCA_BUILD_FFI         OFF CACHE BOOL "" FORCE)

# Pull in OrcaSlicer's tree. EXCLUDE_FROM_ALL means only what slic3r_ffi
# transitively requires gets built — libslic3r, not the OrcaSlicer binary.
add_subdirectory("${ORCA_DIR}" "${CMAKE_BINARY_DIR}/external_OrcaSlicer" EXCLUDE_FROM_ALL)

# The FFI shim itself.
add_library(slic3r_ffi SHARED
    ffi/slic3r_ffi.cpp
    ffi/slic3r_ffi.h
    # Header-only nanosvg implementation. Upstream Orca instantiates it in
    # the GUI layer (gated by SLIC3R_GUI=ON), so we must do it here when
    # building libslic3r without the GUI.
    ffi/nanosvg_impl.cpp
)

set_target_properties(slic3r_ffi PROPERTIES
    PUBLIC_HEADER ffi/slic3r_ffi.h
    SOVERSION    0
    VERSION      0.1.0
)

target_include_directories(slic3r_ffi
    PUBLIC
        $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/ffi>
    PRIVATE
        ${ORCA_DIR}/src
        # nanosvg/nanosvg.h: same path libslic3r picks up via admesh's
        # INTERFACE include dir.
        ${ORCA_DIR}/deps_src
)

target_link_libraries(slic3r_ffi
    PRIVATE
        libslic3r
        boost_libs
)
if (TARGET libslic3r_cgal)
    target_link_libraries(slic3r_ffi PRIVATE libslic3r_cgal)
endif()
