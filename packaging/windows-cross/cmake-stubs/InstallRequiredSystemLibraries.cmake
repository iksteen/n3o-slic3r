# No-op shim that shadows CMake's builtin InstallRequiredSystemLibraries.
#
# The real module's MSVC branch resolves the VC redist directory by querying
# the Windows registry (`cmake_host_system_information(... QUERY VS_<n>_DIR)`),
# which errors when cross-compiling from Linux — there is no registry. Deps that
# `include(InstallRequiredSystemLibraries)` for CPack/packaging (c-blosc) fail
# to configure.
#
# We static-link the deps and ship the CRT via the Tauri app bundle, so we never
# want CMake to auto-bundle the MSVC runtime. Defining the variable the module
# would have produced (empty) keeps any downstream `install(${CMAKE_INSTALL_
# SYSTEM_RUNTIME_LIBS})` harmless.
#
# How it's reached: a dep that needs this resets CMAKE_MODULE_PATH to its own
# `cmake/` dir before the include (so a toolchain-level prepend can't survive),
# so build-deps.sh copies this file *into that dep's module dir* — where the
# dep's own `include()` then finds it instead of the builtin.
set(CMAKE_INSTALL_SYSTEM_RUNTIME_LIBS "")
