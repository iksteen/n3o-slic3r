# CMAKE_PROJECT_INCLUDE — runs right after project()/enable_language(RC), late
# enough that the include dirs land in every target's RC INCLUDES.
#
# Why this exists: the toolchain wires the SDK headers for C/C++ via /imsvc,
# but the .rc compile (CMake preprocesses .rc with `clang-cl -E`, then llvm-rc)
# pulls its includes from the *target* include dirs, not the C/C++ flags — so
# without this, version-info .rc files fail with "'windows.h' file not found"
# (hit on zlib, OCCT, ...). Adding the SDK dirs to include_directories() fixes
# both the preprocess and llvm-rc steps. Harmless for C/C++ (already covered).
set(_X "$ENV{XWIN_DIR}")
include_directories(
  "${_X}/sdk/include/um"
  "${_X}/sdk/include/shared"
  "${_X}/sdk/include/ucrt"
  "${_X}/crt/include")
