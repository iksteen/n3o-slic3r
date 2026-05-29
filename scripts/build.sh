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

case "${1:-deps}" in
    deps)
        if [[ -d "${DEPS_INSTALL}" ]]; then
            echo "deps: already built at ${DEPS_INSTALL}, skipping"
            exit 0
        fi
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
        ;;
    *)
        echo "usage: $0 deps" >&2
        echo "Other build steps are now driven by \`cargo build\`." >&2
        exit 2
        ;;
esac
