#!/usr/bin/env bash
# One-time setup: build OrcaSlicer's heavy dependency tree (Boost / CGAL /
# OCCT / TBB / OpenVDB / ...) into external/OrcaSlicer/deps/build/. Once
# this exists, `cargo build` from the workspace root drives everything
# else (libslic3r, the FFI shim, bindgen, the Rust crates, Tauri).
#
# Usage: ./scripts/build.sh deps
#
# Takes ~17 minutes on a fast machine. Idempotent; re-running is a no-op
# once the deps prefix exists.

set -e

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ORCA_DIR="${REPO_ROOT}/external/OrcaSlicer"
DEPS_INSTALL="${ORCA_DIR}/deps/build/OrcaSlicer_dep/usr/local"
HOST_OS=$(uname -s)
HOST_ARCH=$(uname -m)
DEPS_STAMP=""

deps_ready() {
    local prefix="$1"
    [[ -f "${prefix}/include/mpfr.h" ]] || return 1
    [[ -f "${prefix}/lib/libmpfr.a" || -f "${prefix}/lib/libmpfr.la" || -f "${prefix}/lib/libmpfr.dylib" ]] || return 1
    [[ -f "${prefix}/lib/libgmp.a" || -f "${prefix}/lib/libgmp.la" || -f "${prefix}/lib/libgmp.dylib" ]] || return 1
    [[ -f "${prefix}/lib/libtbb.a" || -f "${prefix}/lib/libtbb.dylib" ]] || return 1
}

mark_deps_complete() {
    local prefix="$1"
    local stamp="$2"
    if deps_ready "${prefix}"; then
        mkdir -p "$(dirname "${stamp}")"
        : > "${stamp}"
    fi
}

case "${1:-deps}" in
    deps)
        DEPS_STAMP="${ORCA_DIR}/deps/build/.n3o-deps-complete"
        if [[ -d "${DEPS_INSTALL}" ]] && deps_ready "${DEPS_INSTALL}"; then
            echo "deps: already built at ${DEPS_INSTALL}, skipping"
            exit 0
        fi
        case "${HOST_OS}" in
            Linux)
                echo "deps: building OrcaSlicer's deps tree (one-time, ~17 min)..."
                # `-r` skips OrcaSlicer's >=10G RAM precheck. The check is a
                # conservative heuristic; GitHub-hosted runners have ~7G RAM +
                # swap and complete the build successfully, just slower.
                #
                # `-fpermissive` added to CXXFLAGS: OCCT
                # (src/StdPrs/StdPrs_BRepFont.cxx:465 +
                # src/GeomToStep/...) has unsigned-char* → const char*
                # implicit conversions that GCC 16 rejects by default.
                # Same workaround packaging/arch/PKGBUILD applies for the
                # Arch package builds. Upstream OCCT code, not ours.
                (cd "${ORCA_DIR}" && \
                 CXXFLAGS="${CXXFLAGS:-} -fpermissive" ./build_linux.sh -d -r)
                mark_deps_complete "${DEPS_INSTALL}" "${DEPS_STAMP}"
                ;;
            Darwin)
                DEPS_INSTALL="${ORCA_DIR}/deps/build/${HOST_ARCH}/OrcaSlicer_dep/usr/local"
                DEPS_STAMP="${ORCA_DIR}/deps/build/${HOST_ARCH}/.n3o-deps-complete"
                if [[ -d "${DEPS_INSTALL}" ]] && deps_ready "${DEPS_INSTALL}"; then
                    echo "deps: already built at ${DEPS_INSTALL}, skipping"
                    exit 0
                fi
                echo "deps: building OrcaSlicer's macOS deps tree for ${HOST_ARCH}..."
                (cd "${ORCA_DIR}" && ./build_release_macos.sh -d -a "${HOST_ARCH}")
                mark_deps_complete "${DEPS_INSTALL}" "${DEPS_STAMP}"
                ;;
            *)
                cat >&2 <<EOF
deps: unsupported host OS: ${HOST_OS}
This wrapper only knows how to bootstrap OrcaSlicer's dependency tree on Linux and macOS.
EOF
                exit 1
                ;;
        esac
        if ! deps_ready "${DEPS_INSTALL}"; then
            echo "deps: dependency tree at ${DEPS_INSTALL} is still incomplete after the build" >&2
            exit 1
        fi
        ;;
    *)
        echo "usage: $0 deps" >&2
        echo "Other build steps are now driven by \`cargo build\`." >&2
        exit 2
        ;;
esac
