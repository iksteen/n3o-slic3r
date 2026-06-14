#!/usr/bin/env bash
# Run a cargo / tauri build for a macOS target from Linux through osxcross.
#
# Sets the osxcross env that three consumers read:
#   - crates/slic3r-ffi/build.rs   (cmake toolchain: OSXCROSS_*, MACCROSS_PREFIX)
#   - cargo's target linker         (CARGO_TARGET_<triple>_LINKER → ld64 wrapper)
#   - the `cc` crate                (CC_/CXX_/AR_<triple> for C deps in the tree)
#
# Usage:  env.sh <arm64|x86_64> <command...>
#   env.sh arm64  cargo build -p n3o-slic3r --target aarch64-apple-darwin --release
#   env.sh arm64  npm run tauri build -- --target aarch64-apple-darwin
#   env.sh x86_64 cargo build -p n3o-slic3r --target x86_64-apple-darwin --release
#
# The deps tree for the arch must already be cross-built
# (./build-deps.sh <arch>); this only wires the toolchain, it does not build deps.
# (build.sh is the full artifact build — deps + frontend + cross build + bundle —
# and uses this wrapper internally.)
set -euo pipefail

ARCH="${1:?usage: env.sh <arm64|x86_64> <command...>}"; shift || true
case "$ARCH" in
  arm64)  RUST_TRIPLE=aarch64-apple-darwin ;;
  x86_64) RUST_TRIPLE=x86_64-apple-darwin ;;
  *) echo "arch must be arm64 or x86_64" >&2; exit 2 ;;
esac
[ "$#" -gt 0 ] || { echo "error: no command given" >&2; exit 2; }

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$here/../.." && pwd)"

# Resolve osxcross (builds it in-tree on first use; honors a preset OSXCROSS_ROOT).
OSXCROSS_ROOT="$("$here/ensure-osxcross.sh")"
export OSXCROSS_ROOT
export PATH="$OSXCROSS_ROOT/bin:$PATH"
command -v osxcross-conf >/dev/null || { echo "error: osxcross-conf not on PATH (OSXCROSS_ROOT=$OSXCROSS_ROOT)" >&2; exit 1; }
eval "$(osxcross-conf)"
export OSXCROSS_TARGET_DIR OSXCROSS_SDK OSXCROSS_TARGET
DARWIN="${OSXCROSS_TARGET#darwin}"
export OSXCROSS_HOST="${ARCH}-apple-darwin${DARWIN}"   # toolchain.cmake reads this to pick the arch

# The osxcross per-arch clang wrapper (links Mach-O via ld64; the aarch64 spelling
# is the automake-friendly alias and is what cmake/automake projects use too).
WRAP_TRIPLE="$([ "$ARCH" = arm64 ] && echo "aarch64-apple-darwin${DARWIN}" || echo "x86_64-apple-darwin${DARWIN}")"
CC_BIN="$OSXCROSS_TARGET_DIR/bin/${WRAP_TRIPLE}-clang"
CXX_BIN="$OSXCROSS_TARGET_DIR/bin/${WRAP_TRIPLE}-clang++"
AR_BIN="$OSXCROSS_TARGET_DIR/bin/${WRAP_TRIPLE}-ar"
[ -x "$CC_BIN" ] || { echo "error: osxcross wrapper missing: $CC_BIN — is osxcross built?" >&2; exit 1; }

# The cross-deps prefix build.rs's cmake toolchain searches (find_root_path).
export MACCROSS_PREFIX="$REPO_ROOT/external/OrcaSlicer/deps/build/${ARCH}/OrcaSlicer_dep/usr/local"
[ -f "$MACCROSS_PREFIX/.deps-complete" ] || echo ":: warning: $MACCROSS_PREFIX has no .deps-complete — run ./build-deps.sh $ARCH first" >&2

# cargo target linker (UPPER_SNAKE of the rust triple).
LINK_VAR="CARGO_TARGET_$(echo "$RUST_TRIPLE" | tr 'a-z-' 'A-Z_')_LINKER"
export "$LINK_VAR=$CC_BIN"
# `cc` crate vars for C/C++ deps compiled for the target. Bash identifiers can't
# contain '-', so use the underscore spelling (the cc crate also looks up
# CC_<triple-with-underscores>, e.g. CC_aarch64_apple_darwin).
TRIPLE_US="${RUST_TRIPLE//-/_}"
export "CC_${TRIPLE_US}=$CC_BIN" "CXX_${TRIPLE_US}=$CXX_BIN" "AR_${TRIPLE_US}=$AR_BIN"

echo ":: macOS cross — arch=$ARCH triple=$RUST_TRIPLE host=$OSXCROSS_HOST SDK=$OSXCROSS_SDK_VERSION"
echo ":: $*"
exec "$@"
