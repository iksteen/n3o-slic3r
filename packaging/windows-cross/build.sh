#!/usr/bin/env bash
# Cross-build the Windows app + NSIS installer from Linux — no Windows host, no
# wine. Ensures the cross-deps tree (build-deps.sh, one-time), then drives
# `cargo xwin` (clang-cl + the xwin MSVC CRT/SDK) for the app and `tauri build`
# (makensis on Linux) for the installer. publish.sh GPG-signs + uploads it.
#
# Prereqs:
#   - `cargo install cargo-xwin`; `rustup target add x86_64-pc-windows-msvc`.
#   - node deps installed (`npm install`) for the frontend + tauri-cli.
#
# Env:
#   WINCROSS_PREFIX  cross-deps prefix.  Default: this dir's .build/prefix.
#   XWIN_DIR         CRT+SDK splat dir.  Default: cargo-xwin's cache.
#   TARGET           rust target triple. Default: x86_64-pc-windows-msvc.
#   BUNDLES          tauri bundle list.  Default: nsis.  Set to '' to skip the
#                    installer (build just the .exe).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
: "${WINCROSS_PREFIX:=$here/.build/prefix}"
: "${XWIN_DIR:=${XDG_CACHE_HOME:-$HOME/.cache}/cargo-xwin/xwin}"
: "${TARGET:=x86_64-pc-windows-msvc}"
: "${BUNDLES:=nsis}"
export WINCROSS_PREFIX XWIN_DIR

# Ensure the cross-deps tree. Gate on the completion stamp, not just lib/: a
# partial/interrupted build-deps run leaves early deps' libs behind, which would
# otherwise sail past here and fail deep in the FFI cmake configure (e.g.
# find_package(Boost) not found). build-deps.sh is the slow one-time step.
if [[ -f "$WINCROSS_PREFIX/.deps-complete" ]]; then
  echo ":: reusing complete cross-deps prefix at $WINCROSS_PREFIX"
else
  echo ":: cross-deps prefix missing or incomplete — building it (one-time, slow)"
  "$here/build-deps.sh"
fi

# tauri execs the runner as a bare `cargo-xwin` binary, so its dir must be on
# PATH — cargo resolves `cargo xwin` subcommands itself, but tauri does not, and
# $CARGO_HOME/bin isn't always on PATH. Prepend it, then verify.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
command -v cargo-xwin >/dev/null || { echo "error: cargo-xwin not installed (cargo install cargo-xwin)." >&2; exit 1; }

echo ":: WINCROSS_PREFIX=$WINCROSS_PREFIX"
echo ":: target=$TARGET  bundles=${BUNDLES:-<none>}"
cd "$root"

if [[ -z "$BUNDLES" ]]; then
  echo ":: building app only (cargo xwin, release)"
  cargo xwin build --release --target "$TARGET" -p n3o-slic3r
  echo ":: -> target/$TARGET/release/n3o-slic3r.exe (+ slic3r_ffi.dll beside it)"
else
  echo ":: building app + installer (tauri build, runner=cargo-xwin)"
  npx tauri build --runner cargo-xwin --target "$TARGET" --bundles "$BUNDLES"
  echo ":: -> target/$TARGET/release/bundle/"
fi
