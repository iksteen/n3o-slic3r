#!/usr/bin/env bash
# Remove the browser demo build output (demo/dist/, git-ignored). The committed
# data assets in demo/assets/ are not touched.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [ -e "${repo}/demo/dist" ]; then
  echo ":: rm -rf demo/dist"
  rm -rf -- "${repo}/demo/dist"
fi

echo ":: demo clean complete."
