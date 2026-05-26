# Phase 7b smoke — Snapmaker U1 real-print walkthrough

The exit gate for Phase 7b. Use this to validate the U1 driver +
panel + cascade on live hardware after any change that could
plausibly affect them (cascade composer, driver protocol, panel
rendering, slice pipeline).

## Pre-flight

- U1 powered on, on the LAN, host reachable.
- All 4 toolheads docked with filament loaded (any combination
  for single-material; differentiated colors/types for multi-mat
  scenarios so the panel + tool changes are visually verifiable).
- The dev binary built from current HEAD: `npm run tauri dev`.

## 1. Connection

1. Bind plate 1 to the U1 via the MaterialBindingPanel.
2. Credentials dialog appears → enter host + port (default 80).
   Click *Test connection*.
3. Confirm `U1ToolheadStrip` lights up with the 4 reported
   filaments (color chips + material labels per cell).

## 2. Single-material print

Goal: confirm the slice → upload → start → finish loop works
without any toolchanges in play.

1. Slot 1: any PLA. Slots 2/3/4: any (won't be used).
2. Load any cube (e.g. `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`).
3. Bind model material 1 → slot 1 (auto-binds on object register;
   confirm in the binding panel).
4. Slice + send. Confirm state → `RUNNING`, T0 stays mounted
   throughout the print, completion → state `FINISH`.

**Validated**: the 20mm cube smoke that the bed-temp fix work
(commits `31c2685`, `16d0e0f`) was originally about — bed reads
55 °C in the live print, T0 mounts and stays put.

## 3. Multi-material print

Goal: confirm tool changes, per-toolhead color rendering,
per-toolhead live temps (current + target), and the active-tool
ring on the panel.

1. Load `src-tauri/tests/fixtures/3mf/four-cubes-4mat.3mf` (4 cubes
   in a 2×2 grid, one per material M1..M4).
2. Auto-bind drops M1→T0, M2→T1, M3→T2, M4→T3 thanks to the
   first-free-from-preferred policy (commit `08ba51b`).
3. Slice. Confirm the preview shows 4 colors.
4. Send. Watch the panel:
   - Color chip per cell matches the loaded filament.
   - Active-toolhead ring moves between T0..T3 as the carriage
     re-docks.
   - Per-toolhead temps tick — including standby drops as toolheads
     park (slicer emits `M104 S<setpoint - standby_delta>` per
     `standby_temperature_delta` = -150 in the snappy process
     fragment; the printer's `change_filament_gcode` then `M109`-waits
     the incoming toolhead to its full setpoint at the swap).
5. Print to completion.

**Validated**: live 4-color print finished successfully.

## 4. Pause / resume / stop

During any active print:

1. Click *Pause*. Confirm state → `PAUSED`; head parks at the
   firmware-side safe position.
2. Click *Resume*. Confirm state → `RUNNING`; print continues
   from the parked position.
3. (Separate run.) Click *Stop*. Confirmation dialog → confirm.
   State → `FAILED`, head parks, bed cools.

**Validated**: all three commands behave correctly on live U1
hardware.

## What this gate does NOT cover

Out of Phase 7b scope; tracked separately:

- **Filament sync** — matching the printer-reported loaded
  filament to the project's material binding. Phase 7c.
- **Mismatch detection** — warn / block on family / temp / color
  mismatch. Phase 7c.
- **Build plate auto-detection** — the U1 doesn't expose its
  loaded plate via Moonraker subscribe set; manual selection only
  (PR-7b-3 known gap).
- **Mixed-nozzle real print** — different nozzle SKUs across
  toolheads. Cascade ticket scoped this out for MVP.

## Diagnostics

When something doesn't work:

- **WebSocket subscribe fails on connect** → visit
  `http://<host>/printer/info` in a browser. If it 404s, Moonraker
  isn't running on the U1; if it returns JSON, the WS path differs
  or the dev binary's URL is wrong.
- **Per-frame protocol noise** → temporarily re-add the
  `dump_frame()` instrumentation in
  `src/core/driver/snapmaker/moonraker.rs` (same pattern used to
  capture the fixtures at `src-tauri/tests/fixtures/u1-moonraker/`).
  Appends every raw text frame to `/tmp/n3o-u1-moonraker-raw.jsonl`.
- **Per-toolhead extra trace** →
  `RUST_LOG=n3o_slic3r_lib::core::driver::snapmaker=trace` in the
  dev binary's env.
- **Tool changes drop filament** → diff the cascade's
  `change_filament_gcode` against Snapmaker Orca's reference
  (`external/OrcaSlicer/resources/profiles/Snapmaker/machine/`).
  May need a `retract_length_toolchange` or `flush_distance` tweak.
