cmake_minimum_required(VERSION 3.13)
project(libslic3r_ffi)

# The flat C API shim. See ../README.md for the design.
#
# Builds as a SHARED library that statically absorbs libslic3r and all its
# private dependencies. PIC is on globally (set in the outer CMakeLists);
# the static archives from external/OrcaSlicer/deps/build link cleanly.

add_library(slic3r_ffi SHARED
    slic3r_ffi.cpp
    slic3r_ffi.h
    # Header-only nanosvg implementation. Upstream Orca instantiates this in
    # src/slic3r/GUI/BitmapCache.cpp, which is gated by SLIC3R_GUI=ON. We
    # build with SLIC3R_GUI=OFF, so libslic3r's nsvg* references would be
    # unresolved without supplying the definitions here.
    nanosvg_impl.cpp
)

set_target_properties(slic3r_ffi PROPERTIES
    PUBLIC_HEADER slic3r_ffi.h
    SOVERSION    0
    VERSION      0.1.0
)

# The OrcaSlicer tree was added with EXCLUDE_FROM_ALL but its targets are
# still visible by name. We reach across to libslic3r for everything.
target_include_directories(slic3r_ffi
    PUBLIC
        $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}>
        $<INSTALL_INTERFACE:include>
    PRIVATE
        ${CMAKE_SOURCE_DIR}/external/OrcaSlicer/src
        # Picks up "nanosvg/nanosvg.h" — same path libslic3r picks up
        # implicitly via admesh's INTERFACE include dir.
        ${CMAKE_SOURCE_DIR}/external/OrcaSlicer/deps_src
)

target_link_libraries(slic3r_ffi
    PRIVATE
        libslic3r
        boost_libs
)
if (TARGET libslic3r_cgal)
    target_link_libraries(slic3r_ffi PRIVATE libslic3r_cgal)
endif()

install(
    TARGETS       slic3r_ffi
    LIBRARY       DESTINATION lib
    ARCHIVE       DESTINATION lib
    PUBLIC_HEADER DESTINATION include
)
