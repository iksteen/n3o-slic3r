#!/usr/bin/env bash
# Ensure an osxcross toolchain (with the macOS SDK packaged in) is available,
# building it in-tree on first use — into the gitignored .build/ scratch, the
# same pattern bundle-app.sh uses for libdmg-hfsplus. This decouples the macOS
# cross build from any hand-built toolchain in $HOME and needs no Mac: the SDK
# is fetched, pinned + checksummed, from a public mirror.
#
# Resolution order:
#   1. $OSXCROSS_ROOT, if it has bin/osxcross-conf — an existing/system install
#      the caller pointed us at (e.g. ~/osxcross/target or /usr/local/osx-ndk-x86,
#      provided it carries a usable SDK).
#   2. the in-tree build at .build/osxcross/target — built here on first use.
#
# Echoes the resolved OSXCROSS_ROOT on stdout; all progress goes to stderr, so
# callers can do:  OSXCROSS_ROOT="$(ensure-osxcross.sh)"
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Pinned, reproducible inputs.
OSXCROSS_REPO="https://github.com/tpoechtrager/osxcross"
OSXCROSS_COMMIT="e6ab3fa7423f9235ce9ed6381d6d3af191b46b59"
# Public packaged macOS 15.5 SDK (joseluisq/macosx-sdks), pinned by sha256. The
# SDK is Apple's and not redistributable by Apple; this is a community mirror —
# use only if you hold a macOS license. Swap the URL+sha to change SDK version.
SDK_URL="https://github.com/joseluisq/macosx-sdks/releases/download/15.5/MacOSX15.5.sdk.tar.xz"
SDK_SHA256="c15cf0f3f17d714d1aa5a642da8e118db53d79429eb015771ba816aa7c6c1cbd"

# 1. An explicitly-provided (or system) install wins.
if [ -n "${OSXCROSS_ROOT:-}" ] && [ -x "${OSXCROSS_ROOT}/bin/osxcross-conf" ]; then
  echo "${OSXCROSS_ROOT}"; exit 0
fi

src="${here}/.build/osxcross"
root="${src}/target"
if [ -x "${root}/bin/osxcross-conf" ]; then
  echo "${root}"; exit 0
fi

# 2. Build it in-tree (one-time).
{
  for t in git curl cmake make clang clang++ patch tar xz python3 sha256sum; do
    command -v "$t" >/dev/null || { echo "error: building osxcross needs '$t'" >&2; exit 1; }
  done

  echo ":: building osxcross in-tree (${src}) — one-time, ~5-10 min" >&2
  # Shallow-fetch the pinned commit (GitHub allows fetching a SHA directly).
  if [ ! -d "${src}/.git" ]; then
    mkdir -p "${src}"
    git -C "${src}" init -q
    git -C "${src}" remote add origin "${OSXCROSS_REPO}"
  fi
  git -C "${src}" fetch -q --depth 1 origin "${OSXCROSS_COMMIT}"
  git -C "${src}" checkout -q "${OSXCROSS_COMMIT}"

  # Pinned, checksummed SDK into osxcross's tarballs/ (build.sh picks it up).
  mkdir -p "${src}/tarballs"
  sdk="${src}/tarballs/$(basename "${SDK_URL}")"
  if [ ! -f "${sdk}" ] || ! echo "${SDK_SHA256}  ${sdk}" | sha256sum -c - >/dev/null 2>&1; then
    echo ":: downloading $(basename "${SDK_URL}")" >&2
    curl -fsSL -o "${sdk}" "${SDK_URL}"
    echo "${SDK_SHA256}  ${sdk}" | sha256sum -c - >&2
  fi

  ( cd "${src}" && UNATTENDED=1 ./build.sh )
} >&2

[ -x "${root}/bin/osxcross-conf" ] || {
  echo "error: osxcross build did not produce ${root}/bin/osxcross-conf" >&2; exit 1
}
echo "${root}"
