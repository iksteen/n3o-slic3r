# Wave-overhang carry

Carries the **wave-overhang** toolpath generator (support-free steep overhangs via
curved, laterally-anchored wavefronts) from
[dennisklappe/OrcaSlicer-WaveOverhangs](https://github.com/dennisklappe/OrcaSlicer-WaveOverhangs)
onto our pinned `external/OrcaSlicer` submodule as a build-time patch set — the
same model as `packaging/windows-cross/patches/` (apply over the clean upstream
pin, never commit into the submodule).

It is a **libslic3r engine** feature. The fork's GUI tab is irrelevant — n3o's
settings panel is data-driven off `PrintConfigDef`, so the ~40 `wave_overhang_*`
process options surface automatically once they exist in the vendored libslic3r
(re-run `scripts/scrape_option_buckets.py` + `scripts/scrape_option_display_order.py`).
The generator runs inside `Print::process()` gated on `wave_overhangs`, so no FFI
change is needed.

## Base

The patches are rebased onto OrcaSlicer pin **`6bb7903b`** (2026-06-11, `origin/main`).
The fork's own base was older (`3e4af2c`, 2026-04-13); the delta below includes
the conflict resolutions, the 2D→3D `Polyline3` adaptation, and the
`wall_filament`→`outer_wall_filament_id` / `role_speed` re-anchoring that bumping
to this pin required. Verified: `cargo build -p slic3r-ffi` links cleanly against
this base with the carry applied.

## Patches

- **`0001-wave-overhangs-module.patch`** — the new `src/libslic3r/WaveOverhangs/`
  module (5 files, pure adds). Touches no upstream lines, so it applies regardless
  of how far the pin moves.
- **`0002-wave-overhangs-hooks.patch`** — additive hooks into 22 existing files
  (the `wave_overhang_*` PrintConfig options, the PerimeterGenerator/PrintObject
  invocation, GCode/CoolingBuffer overrides, `Preset::print_options` registration,
  ExtrusionPath path-tag fields). **This is the bump-fragile surface** — re-validate
  it on every submodule bump.

## Apply

```sh
crates/slic3r-ffi/patches/wave-overhangs/apply.sh   # idempotent; errors if the pin moved
cargo build -p slic3r-ffi                            # rebuilds libslic3r with the module
```

## Maintenance

The carry must be re-validated whenever **our pin** *or* **the upstream fork**
moves. Two distinct failure modes on a bump:

1. **Context drift** (patch-apply level) — `0002` hunks fail because upstream
   churned the lines they sit next to. Regenerate by re-merging the fork onto the
   new pin and re-resolving conflicts (last time: 5 files, all trivial unions, over
   a 6-week / 380-commit gap).
2. **API drift** (compile level) — a patch applies but won't compile because an API
   the wave code calls changed. Last time this was a single concept: `ExtrusionPath`
   went 2D `Polyline` → 3D `Polyline3` (~7 sites: `Polyline3(poly)` to lift,
   `.to_polyline()` to project). Bounded and mechanical, but it recurs per bump.

### Regenerate

```sh
cd external/OrcaSlicer
git fetch https://github.com/dennisklappe/OrcaSlicer-WaveOverhangs.git main
W=$(git merge-base HEAD FETCH_HEAD)                 # the fork's base
# build a "(merge-base + wave)" commit, merge it onto the new pin, fix drift, then:
P=../../crates/slic3r-ffi/patches/wave-overhangs/patches
git -c diff.noprefix=false diff <newpin>..<resolved> -- src/libslic3r/WaveOverhangs/ \
  > "$P/0001-wave-overhangs-module.patch"
git -c diff.noprefix=false diff <newpin>..<resolved> -- src/libslic3r/ \
  ':(exclude)src/libslic3r/WaveOverhangs/' \
  > "$P/0002-wave-overhangs-hooks.patch"
```

If bumps become frequent, switch the carry to a fork branch (submodule → our
OrcaSlicer branch = pin + wave commits): `git rebase` handles the hook drift with
real ancestry instead of patch fuzz.
