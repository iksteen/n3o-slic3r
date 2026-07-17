#!/usr/bin/env bash
#
# Pin a new OrcaSlicer revision and regenerate everything we derive from it: the
# scraped option tables and every bundled printer, quality (process) and filament
# profile — the tedious half of the OrcaSlicer pin-bump, captured in one place.
#
#   scripts/sync_orcaslicer.sh [<orca-ref>]
#
# With <orca-ref> (a commit / tag / branch), the external/OrcaSlicer submodule is
# reset (dropping the in-place build patches, which slic3r-ffi/build.rs re-applies
# at the next `cargo build`), fetched, and checked out to it. Without it, the
# currently-checked-out revision is used.
#
# It then builds OrcaSlicer's deps tree (idempotent) and runs the full test
# suite against the pin — after a repin it forces a real libslic3r recompile
# first, since upstream option renames surface as test breakage. Everything is
# regenerated in place and left STAGED FOR REVIEW — the script does not commit.
# Eyeball the profile + scraper diffs, then commit the submodule bump alongside
# them. See docs/dev/profiles.md for the rest of the pin-bump notes.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

ORCA_SUB="external/OrcaSlicer"
ORCA_PROFILES="$ORCA_SUB/resources/profiles"
ref="${1:-}"

if [[ -n "$ref" ]]; then
  echo "==> Pinning $ORCA_SUB to '$ref'"
  # The submodule's tracked modifications are build-time carry patches
  # (wave-overhangs + cross-compile), all regenerable — drop them so the new ref
  # checks out clean. Untracked build artifacts are left alone.
  git -C "$ORCA_SUB" reset --hard --quiet HEAD
  # `--force` so a moved tag (OrcaSlicer's `nightly-builds`) updates instead of
  # rejecting the whole fetch. If the quiet fetch fails, re-run it loud so the
  # real error surfaces (and set -e aborts) rather than being swallowed.
  git -C "$ORCA_SUB" fetch --tags --force --quiet origin \
    || git -C "$ORCA_SUB" fetch --tags --force origin
  git -C "$ORCA_SUB" checkout --quiet "$ref"
  git -C "$ORCA_SUB" submodule update --init --recursive --quiet
fi
echo "==> OrcaSlicer at $(git -C "$ORCA_SUB" rev-parse --short HEAD)"

# The wave-overhang carry adds config options / UI rows, so the option scrapers
# must see it. apply.sh is idempotent (skips an already-patched tree).
echo "==> Applying the wave-overhang carry (for the scrapers)"
crates/slic3r-ffi/patches/wave-overhangs/apply.sh

# --- Option scrapers: UI display order + printer/filament page layout, read
#     from OrcaSlicer's Tab.cpp. Output is committed Rust tables. ---
echo "==> Scraping option layout"
python3 scripts/scrape_option_display_order.py
python3 scripts/scrape_option_printer_pages.py

# --- Machines: one base-machine + per-nozzle family per printer. Add a printer
#     by adding a line: vendor model slug toml-out [skip-SKUs]. `skip` drops
#     mixed-nozzle leaves (e.g. the U1's "0.4+0.6") that aren't real variants. ---
machine() { # vendor model slug toml-out [skip] [pick]
  local extra=()
  [[ -n "${5:-}" ]] && extra+=(--skip "$5")
  [[ -n "${6:-}" ]] && extra+=(--pick "$6")
  python3 scripts/import_machine_profile.py \
    --root "$ORCA_PROFILES" --vendor "$1" --model "$2" --slug "$3" \
    --toml-out "$4" "${extra[@]}"
}
echo "==> Importing machines"
# P1S/P1P embed the nozzle SKU in the machine_start_gcode header comment, so
# it disagrees across nozzle variants — cosmetic, so --pick it (ship canonical).
machine BBL       "Bambu Lab A1 mini" bambu-lab-a1-mini resources/profiles/bbl/printer
machine BBL       "Bambu Lab A1"      bambu-lab-a1      resources/profiles/bbl/printer
machine BBL       "Bambu Lab P1S"     bambu-lab-p1s     resources/profiles/bbl/printer "" machine_start_gcode
machine BBL       "Bambu Lab P1P"     bambu-lab-p1p     resources/profiles/bbl/printer "" machine_start_gcode
machine Snapmaker "Snapmaker U1"      snapmaker-u1      resources/profiles/snapmaker/printer "0.4+0.6"
machine Creality  "Creality Ender-3 S1" creality-ender-3-s1 resources/profiles/creality/printer
machine Creality  "Creality Ender-3 V3 KE" creality-ender-3-v3-ke resources/profiles/creality/printer

# --- Engine resources: files libslic3r reads off its own resources_dir()
#     at runtime (nozzle-HRC table for the abrasive-filament check).
#     Shipped under resources/engine/, passed to slic3r_init. ---
echo "==> Copying engine resources"
mkdir -p resources/engine/info
cp "$ORCA_SUB/resources/info/nozzle_info.json" resources/engine/info/nozzle_info.json

# --- Processes: auto-discovers the printers above from their machine.toml and
#     folds every compatible upstream process leaf. Must run AFTER the machines. ---
echo "==> Importing processes"
python3 scripts/import_processes.py --root "$ORCA_PROFILES" --profiles-root resources/profiles

# --- Derived variants: printers we ship that upstream doesn't (e.g. Klipper
#     conversions of Marlin machines). Regenerated from the freshly-imported
#     base profile with declared patches; each variant's model.toml stays
#     hand-curated. Must run AFTER machines + processes (it copies both). ---
echo "==> Deriving printer variants"
python3 scripts/derive_printer_variant.py \
  --base resources/profiles/creality/printer/creality-ender-3-s1 \
  --slug creality-ender-3-s1-klipper \
  --model-suffix " (Klipper)" \
  --set gcode_flavor=klipper

# --- Filaments: one branded (or cross-vendor Generic) consolidation per bucket.
#     Add a bucket by adding a line: out-dir brand (the filament_vendor value). ---
filament() { # out brand
  python3 scripts/import_filaments.py --root "$ORCA_PROFILES" --out "$1" --brand "$2"
}
echo "==> Importing filaments"
filament resources/profiles/generic/filament   Generic
filament resources/profiles/bbl/filament        "Bambu Lab"
filament resources/profiles/snapmaker/filament  Snapmaker

# --- Build OrcaSlicer's deps tree (idempotent — skips when the prefix exists,
#     first build is ~17 min) and run the full suite against the pin. After a
#     repin, force build.rs to recompile libslic3r: its rerun-if-changed watches
#     the shim + patches, NOT the submodule source, so cargo would otherwise link
#     the stale engine and false-green. Release FFI config keeps that optimized. ---
echo "==> Building OrcaSlicer deps"
scripts/build.sh deps
if [[ -n "$ref" ]]; then
  touch crates/slic3r-ffi/ffi/slic3r_ffi.cpp
fi
echo "==> Building + testing against the pin"
N3O_SLIC3R_FFI_CMAKE_CONFIG=Release cargo test --workspace

echo
echo "==> Done. Review, then commit the submodule bump + regenerated files together:"
git status --short -- "$ORCA_SUB" resources/profiles src-tauri || true
