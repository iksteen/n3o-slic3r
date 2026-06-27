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

# AddressSanitizer build option — enable with `-DSLIC3R_FFI_ASAN=ON`.
# Applied before `add_subdirectory(${ORCA_DIR})` so libslic3r inherits
# the flags too (uninstrumented dependencies can't be checked). Use a
# separate build directory (`build/slic3r-ffi-asan/`) to keep the
# instrumented .so out of the normal cargo cache; load it at runtime
# via `LD_LIBRARY_PATH` over the canonical build.
#
# Stack traces benefit from `-fno-omit-frame-pointer`;
# `-fno-optimize-sibling-calls` keeps tail-call elision from collapsing
# frames ASan needs.
option(SLIC3R_FFI_ASAN "Build with AddressSanitizer" OFF)
if(SLIC3R_FFI_ASAN)
    message(STATUS "slic3r-ffi: AddressSanitizer ENABLED")
    add_compile_options(-fsanitize=address -fno-omit-frame-pointer -fno-optimize-sibling-calls)
    add_link_options(-fsanitize=address)
endif()

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

# Note: OrcaSlicer's CMakeLists.txt:947 hardcodes
# `set(CPACK_RESOURCE_FILE_LICENSE ${CMAKE_SOURCE_DIR}/LICENSE.txt)`
# inside the add_subdirectory scope, which overrides any parent-scope
# value. We work around this with a symlink LICENSE.txt → ../../LICENSE.txt
# at this directory so CPack finds the file without duplication.

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

# build.rs passes the pinned OrcaSlicer submodule SHA so slic3r_version() can
# report exactly which engine revision the shim links. Optional — omitted when
# building outside a git checkout (the C++ guards on #ifdef N3O_ORCA_SHA).
if (N3O_ORCA_SHA)
    target_compile_definitions(slic3r_ffi PRIVATE N3O_ORCA_SHA=${N3O_ORCA_SHA})
endif()

# Linux→macOS osxcross cross only: supply __isPlatformVersionAtLeast (the
# compiler-runtime helper @available lowers to) that Apple's clang_rt would
# otherwise provide. See ffi/macos_availability_shim.mm. build.rs sets
# N3O_MACOS_CROSS exclusively for that host/target combination.
if (N3O_MACOS_CROSS)
    target_sources(slic3r_ffi PRIVATE ffi/macos_availability_shim.mm)
endif()

# On Windows (MSVC/clang-cl) the shim needs the same baseline defines libslic3r
# compiles with — its public headers (calib.hpp, …) use M_PI (needs
# _USE_MATH_DEFINES; MSVC's <cmath> doesn't expose it like glibc) and would trip
# the min/max macros from <windows.h> (NOMINMAX). No effect on the Linux build.
if (WIN32)
    target_compile_definitions(slic3r_ffi PRIVATE _USE_MATH_DEFINES NOMINMAX)
    # The slic3r_ffi.h C API carries no __declspec(dllexport) — on Linux the .so
    # exports all public symbols by default. Mirror that on Windows so the
    # slic3r_* entry points land in the DLL's export table (and import lib).
    set_target_properties(slic3r_ffi PROPERTIES WINDOWS_EXPORT_ALL_SYMBOLS ON)
endif()

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

# libslic3r's Emboss and OCCT's TKService reference FreeType. Upstream only
# links freetype for the OCCT dependency on non-Windows — src/libslic3r/
# CMakeLists.txt guards it behind `if(NOT WIN32)`, assuming a stock Windows
# build's OCCT package carries freetype itself. Our windows-msvc cross build
# links a *static* OCCT that doesn't propagate it, so the FT_* symbols are
# unresolved at the shim's DLL link. Pull freetype in here for Windows only
# (the non-Windows builds already get it transitively from libslic3r).
if (WIN32)
    find_package(Freetype REQUIRED)
    target_link_libraries(slic3r_ffi PRIVATE Freetype::Freetype)
endif()
