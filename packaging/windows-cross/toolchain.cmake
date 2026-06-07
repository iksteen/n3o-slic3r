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

# Shadow CMake's builtin InstallRequiredSystemLibraries with a no-op — its MSVC
# branch queries the Windows registry for the VC redist dir, which can't work
# cross (hit by OrcaSlicer's top-level CMakeLists and others). Prepend so our
# stub wins. Survives a project's `list(APPEND CMAKE_MODULE_PATH …)`; a project
# that *replaces* the path (c-blosc) needs the stub copied into its own module
# dir instead — see build-deps.sh. cmake-stubs/.
list(PREPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_LIST_DIR}/cmake-stubs")

if(NOT DEFINED ENV{XWIN_DIR})
  message(FATAL_ERROR "XWIN_DIR not set — the MSVC CRT+SDK splat dir.")
endif()
set(_X "$ENV{XWIN_DIR}")

set(CMAKE_C_COMPILER clang-cl)
set(CMAKE_CXX_COMPILER clang-cl)
set(CMAKE_AR llvm-lib)
set(CMAKE_LINKER lld-link)
# llvm-rc via a wrapper that sets the cp1252 input codepage, so a Latin-1 © in
# a dep's VERSIONINFO (OCCT, FreeType) doesn't error. See llvm-rc-cp1252.
set(CMAKE_RC_COMPILER "${CMAKE_CURRENT_LIST_DIR}/llvm-rc-cp1252")
# xwin ships only the RELEASE CRT (no msvcrtd.lib), so force the release
# runtime (/MD) for every config. CMP0091=NEW makes CMAKE_MSVC_RUNTIME_LIBRARY
# actually take effect even on deps with an old cmake_minimum (else a Debug
# try-compile uses /MDd and fails to link msvcrtd.lib).
set(CMAKE_POLICY_DEFAULT_CMP0091 NEW)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
set(CMAKE_TRY_COMPILE_CONFIGURATION Release)

# clang-cl gets the CRT/SDK headers as *system* includes (/imsvc). The .rc
# step does NOT — that's handled separately by rc-sdk-includes.cmake.
set(_inc "/imsvc${_X}/crt/include \
/imsvc${_X}/sdk/include/ucrt /imsvc${_X}/sdk/include/um \
/imsvc${_X}/sdk/include/shared /imsvc${_X}/sdk/include/winrt")
set(_common "--target=x86_64-pc-windows-msvc -Wno-unused-command-line-argument \
-fuse-ld=lld-link ${_inc}")
# clang 19+ promoted several legacy-C warnings to hard errors by default;
# old C deps (boost.container's dlmalloc, …) trip them. Downgrade for C only —
# the clang analogue of OrcaSlicer's `-fpermissive` GCC workaround for OCCT.
set(_c_legacy "-Wno-error=incompatible-pointer-types \
-Wno-error=implicit-function-declaration -Wno-error=int-conversion")
set(CMAKE_C_FLAGS_INIT "${_common} ${_c_legacy}")
set(CMAKE_CXX_FLAGS_INIT "${_common} /EHsc")

# ASM language, same toolchain + target triple as C/CXX. Needed by deps that
# hand-write assembly — Boost.Context (pulled into the dep closure) switches
# stacks in asm. For the MSVC/PE target its CMake would pick MASM (.asm needing
# ml64, which we don't have cross); build-deps.sh instead selects the GAS-syntax
# sources (BOOST_CONTEXT_ASSEMBLER=gas), which clang-cl's integrated assembler
# builds directly. Harmless for deps that enable no ASM.
set(CMAKE_ASM_COMPILER clang-cl)
set(CMAKE_ASM_FLAGS_INIT "${_common}")

# Bundled deps (clipper2, …) compile with /WX or -Werror and are clean under
# cl.exe but not clang-cl's stricter warning set. A launcher that inserts
# -Wno-error just before `-c` keeps warnings non-fatal — matching the cl.exe
# build. (A plain flag can't win: override.cmake's `-c -- <src>` would treat an
# appended flag as a source file.) See clang-cl-nowerror.
set(CMAKE_C_COMPILER_LAUNCHER "${CMAKE_CURRENT_LIST_DIR}/clang-cl-nowerror")
set(CMAKE_CXX_COMPILER_LAUNCHER "${CMAKE_CURRENT_LIST_DIR}/clang-cl-nowerror")

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
