#!/usr/bin/env bash
# Build orca-slicer-ffi end-to-end: OrcaSlicer's deps tree (if needed), the
# shim shared library, then the Rust smoke tests.
#
# Usage:
#   ./scripts/build.sh              # configure + build slic3r_ffi (assumes deps already built)
#   ./scripts/build.sh deps         # build OrcaSlicer's deps tree (~30 min, one-time)
#   ./scripts/build.sh smoke        # cargo run the introspect + slice examples
#   ./scripts/build.sh all          # deps (if missing) + shim + smoke

set -e

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ORCA_DIR="${REPO_ROOT}/external/OrcaSlicer"
DEPS_INSTALL="${ORCA_DIR}/deps/build/OrcaSlicer_dep/usr/local"

step_deps() {
    if [[ -d "${DEPS_INSTALL}" ]]; then
        echo "deps: already built at ${DEPS_INSTALL}, skipping"
        return
    fi
    echo "deps: building OrcaSlicer's deps tree (one-time, ~30 min)..."
    (cd "${ORCA_DIR}" && ./build_linux.sh -d)
}

step_build() {
    cmake -S "${REPO_ROOT}" -B "${REPO_ROOT}/build" -G "Ninja Multi-Config"
    cmake --build "${REPO_ROOT}/build" --config RelWithDebInfo --target slic3r_ffi
}

step_smoke() {
    (cd "${REPO_ROOT}/bindings/rust" && cargo run --release --example introspect | tail -5)
    echo "--- slice smoke test ---"
    local model="${ORCA_DIR}/tests/data/test_stl/ASCII/20mmbox-LF.stl"
    (cd "${REPO_ROOT}/bindings/rust" && cargo run --release --example slice -- "${model}" /tmp/orca-ffi-smoke.gcode 2>&1 | tail -3)
    ls -lh /tmp/orca-ffi-smoke.gcode
}

case "${1:-build}" in
    deps)  step_deps ;;
    build) step_build ;;
    smoke) step_smoke ;;
    all)   step_deps && step_build && step_smoke ;;
    *)     echo "usage: $0 [deps|build|smoke|all]" >&2; exit 2 ;;
esac
