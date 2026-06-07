# CMake toolchain — cross-compile to x86_64-pc-windows-msvc from Linux with
# clang-cl + LLD, against the MSVC CRT + Windows SDK that cargo-xwin (or the
# standalone `xwin` tool) fetches. This is the *full cross* path: no Windows
# host, no wine, real MSVC ABI (so the output links against cl.exe-built libs).
#
# Pair with this directory's `override.cmake` (CMAKE_USER_MAKE_RULES_OVERRIDE)
# and `rc-sdk-includes.cmake` (CMAKE_PROJECT_INCLUDE) — see README.md for why.
#
# Env:
#   XWIN_DIR         CRT+SDK splat dir (cargo-xwin's `xwin` cache, or an
#                    `xwin splat` output).
#   WINCROSS_PREFIX  install prefix of already-built cross deps; added to the
#                    find-root so find_package resolves them and NOT host libs.

set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR AMD64)

if(NOT DEFINED ENV{XWIN_DIR})
  message(FATAL_ERROR "XWIN_DIR not set — the MSVC CRT+SDK splat dir.")
endif()
set(_X "$ENV{XWIN_DIR}")

set(CMAKE_C_COMPILER clang-cl)
set(CMAKE_CXX_COMPILER clang-cl)
set(CMAKE_AR llvm-lib)
set(CMAKE_LINKER lld-link)
set(CMAKE_RC_COMPILER llvm-rc)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)

# clang-cl gets the CRT/SDK headers as *system* includes (/imsvc). The .rc
# step does NOT — that's handled separately by rc-sdk-includes.cmake.
set(_inc "/imsvc${_X}/crt/include \
/imsvc${_X}/sdk/include/ucrt /imsvc${_X}/sdk/include/um \
/imsvc${_X}/sdk/include/shared /imsvc${_X}/sdk/include/winrt")
set(_common "--target=x86_64-pc-windows-msvc -Wno-unused-command-line-argument \
-fuse-ld=lld-link ${_inc}")
set(CMAKE_C_FLAGS_INIT "${_common}")
set(CMAKE_CXX_FLAGS_INIT "${_common} /EHsc")

set(_libs "-libpath:\"${_X}/crt/lib/x86_64\" \
-libpath:\"${_X}/sdk/lib/um/x86_64\" -libpath:\"${_X}/sdk/lib/ucrt/x86_64\"")
set(CMAKE_EXE_LINKER_FLAGS_INIT "/manifest:no ${_libs}")
set(CMAKE_SHARED_LINKER_FLAGS_INIT "/manifest:no ${_libs}")
set(CMAKE_MODULE_LINKER_FLAGS_INIT "/manifest:no ${_libs}")

# Cross find-root hygiene — only the cross-deps prefix + the SDK, never the
# host. Without this, find_package() happily picks up /usr/include and /usr/lib
# (e.g. the host zlib), dragging host glibc headers into a windows-msvc build.
set(CMAKE_FIND_ROOT_PATH "$ENV{WINCROSS_PREFIX}" "${_X}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
