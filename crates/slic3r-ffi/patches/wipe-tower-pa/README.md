# wipe-tower-pa carry patch

Ports Snapmaker's Orca-fork behaviour: during prime-tower ramming, set a fixed
pressure advance (`ramming_pressure_advance_value`) instead of upstream's `0`.
Upstream leaves the nozzle unpressurized during ramming, which blobs the prime.

## What it changes
- `PrintConfig.{hpp,cpp}`: two new `GCodeConfig` options —
  `enable_change_pressure_when_wiping` (Bool, default false) and
  `ramming_pressure_advance_value` (Float, default 0).
- `GCode/WipeTower2.{hpp,cpp}`: a `disable_linear_advance_value(float)` writer
  variant, called at the two ramming sites when
  `enable_change_pressure_when_wiping` is set (else the old zeroing).

Inert unless the printer profile sets `enable_change_pressure_when_wiping` — only
the Snapmaker U1 does (see its `machine.override.toml`).

## Apply / maintenance
`build.rs` applies `patches/*.patch` to the pinned OrcaSlicer submodule on every
build (idempotent: an applied patch is detected by a reverse-check and skipped).
`./apply.sh` does the same for manual runs. If the pin moves and the patch no
longer applies, regenerate it against the new base (git diff of the four files
above), keeping only these changes.
