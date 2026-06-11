#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
cd "$root"

echo ":: building macOS app + DMG"
npx tauri build --bundles dmg
echo ":: -> src-tauri/target/release/bundle/dmg/"
