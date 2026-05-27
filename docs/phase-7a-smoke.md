# Phase 7a smoke — Bambu A1 mini real-print walkthrough

The exit gate for Phase 7a. Use this to validate the Bambu driver +
panel + AMS lite handling + send pipeline on live hardware after any
change that could plausibly affect them (driver protocol, MQTT topic
shape, AMS slot read, `.gcode.3mf` packaging, send command).

Sibling to `docs/phase-7b-smoke.md` (Snapmaker U1 walkthrough);
together they pin the two MVP drivers.

## Pre-flight

- A1 mini powered on, on the same LAN as the dev machine, host
  reachable. **LAN-only mode** enabled on the printer
  (Settings → Network → "LAN-only mode" toggle).
- AMS lite mounted with ≥1 spool loaded for single-material flows;
  ≥2 spools (differentiated colors) for multi-material flows.
- External spool stand assembled with ≥1 spool fed through the
  printer's `Ext` (back/direct) PTFE for the external-spool flows.
- Credentials at hand:
  - **host**: printer IP or `printer.local` mDNS
  - **access code**: from printer LCD → Settings → Network →
    "LAN access code" (8-digit numeric)
  - **serial**: from printer device label or LCD → About
- The dev binary built from current HEAD: `npm run tauri dev`.

## 1. Connection

1. Launch the app. Bind plate 1 to the A1 mini via the
   `MaterialBindingPanel`.
2. `PrinterCredentialsDialog` appears — enter host, access code,
   serial. Click *Test connection*. Wait for green check (MQTT
   handshake + status subscribe succeed).
3. `BambuPanel` mounts. Verify:
   - **Connected** status badge.
   - Current temps reasonable (≤30 °C on a cold printer; ambient
     reading is expected).
   - **AMS strip** shows the loaded spool(s) — color chip + material
     label per occupied slot. Empty slots render as ghost cells.

## 2. Single-color print

Goal: confirm the slice → upload → start → cancel → re-send → finish
loop works end-to-end without any AMS slot swaps in play.

1. Load `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`
   onto plate 1.
2. Bind model material 1 to the AMS slot that has filament loaded
   (auto-bind plants this on object register — confirm in the
   binding panel; override if you want a specific slot).
3. Slice. Wait for `PlateFinished`.
4. Click *Send*. The panel shows "uploading…" briefly, then state
   transitions to `RUNNING`.
5. Watch the panel update layer-by-layer for the first ~3 layers.
   Verify temps climb toward target (bed 55–65 °C, hotend 215 °C
   for default PLA).
6. Click *Stop*. Confirm dialog → confirm. State transitions to
   `FAILED` (Bambu firmware reports cancel as a failure code; the
   panel surfaces this verbatim).
7. Re-slice + re-send. Let it print to completion.
8. Verify state ends at `FINISH` and the AMS strip's active-slot
   indicator clears.

**Validated**: single-color slice + send + cancel + re-send + finish
loop works on live A1 mini hardware.

## 3. Multi-color AMS print

Goal: confirm AMS slot swaps during a single print, per-slot color
rendering on the panel, and the active-slot ring tracking the
firmware-reported active slot.

1. Load `examples/spike3/fourcolor.3mf` onto a fresh plate.
2. Bind the plate to the A1 mini (re-uses the connection from §1 —
   no second credentials dialog).
3. AMS lite needs ≥2 distinct spools loaded. Bind M1 → AMS slot 1,
   M2 → slot 2, etc. Auto-bind handles this if the slot order
   matches the material indices; override otherwise.
4. Slice. Confirm a tool-change emission in the preview (the
   `Tool` color mode lights up multiple colors at the swap
   boundaries).
5. Send. Watch for AMS swaps mid-print:
   - The active-slot ring in the AMS strip moves between slots as
     the firmware loads each material in turn.
   - The panel's current-filament chip updates per swap.
   - The printer's own AMS lite mechanism cycles audibly (servo
     spool changes).
6. Print to completion.

**Validated**: the 4-color benchy prints with the expected swap
cadence; the panel mirrors the firmware-reported active slot
throughout.

## 4. External-spool only

Goal: confirm a single-material print sourced entirely from the
external spool (not AMS) routes correctly through the send pipeline.

1. Load any single-material model (e.g. the 20mm cube from §2)
   onto a fresh plate.
2. Bind plate to the A1 mini.
3. In the binding panel, bind model material 1 to the **Ext** slot
   (external/direct-feed, the 5th flat-slot index after the 4 AMS
   slots).
4. Slice + send. Verify:
   - `ams_bindings` in the wrapped `.gcode.3mf` is **empty** for
     this material — Ext routes via the `ams_mapping` `-1` sentinel,
     not via an AMS slot. (Regression test for this lives in
     `core::slice::pre_slice_gate` since commit `9458e56`.)
   - `use_ams = false` in the project_file MQTT publish.
   - The printer doesn't try to load from AMS; it pulls from the
     external spool directly.
5. Print completes.

**Validated**: external-spool-only print works without AMS
involvement; the `-1` sentinel routing holds.

## 5. External-spool + 1 AMS slot (mixed)

Goal: confirm a 2-material print with one material on Ext and one
on AMS — the hybrid mode that distinguishes A1 mini's external
spool from a pure secondary feed.

1. Load `src-tauri/tests/fixtures/3mf/cube-halves-2mat.3mf` onto a
   fresh plate (single 20mm cube split into two volumes — lower
   half = M1, upper half = M2; grouped via BBS `<components>` so
   libslic3r reads it as one ModelObject with two ModelVolumes).
2. Bind plate to the A1 mini.
3. In the binding panel:
   - M1 → **Ext** slot (external spool)
   - M2 → an AMS slot with a contrasting color
4. Slice + send. Verify:
   - `ams_bindings` contains **only M2's entry** — M1 is omitted
     because it's Ext-fed.
   - `use_ams = true` (at least one material is AMS-fed → the
     printer's AMS subsystem stays online for the print).
5. During print, the printer alternates between the external spool
   and the AMS slot at the layer-10 transition. Whether manual
   loading/unloading of the external spool is required depends on
   firmware behavior — observe what happens and note it here:

   > **Observed on A1 mini (May 2026)**: firmware handles the
   > source switch automatically. No manual loading/unloading
   > required between layers. The AMS retracts, the external spool
   > primes, the swap completes in a few seconds, the print
   > resumes cleanly.

6. Print completes with the two-tone cube as expected.

**Validated**: hybrid AMS + external print works; the mixed-feed
gate (previously over-restrictive, removed in the same session as
this walkthrough) was a false constraint — A1 mini firmware
genuinely handles AMS ↔ Ext swaps in one job.

## 6. Pause / resume / stop

Tested incidentally during the runs above; not a gating leg here
(PR-7a-6's unit tests cover the command publish shape).

During any active print:

1. Click *Pause*. Confirm state → `PAUSED`; head parks at the
   firmware-side safe position.
2. Click *Resume*. Confirm state → `RUNNING`; print continues from
   the parked position.
3. (Separate run.) Click *Stop*. Confirmation dialog → confirm.
   State → `FAILED`, head parks, bed cools.

## What this gate does NOT cover

Out of Phase 7a scope; tracked separately:

- **Pause / resume from the app** as a gated leg — exercised
  opportunistically (§6) but not bound to ticket closure. PR-7a-6
  unit tests cover the command publish shape.
- **Per-slot filament family / temp / color mismatch detection** —
  Phase 7c.
- **Send-time AMS binding emission into the `.gcode.3mf`** —
  Phase 7c-7. Until that lands, the printer uses the AMS bindings
  the slicer baked in (which match whatever the user manually set
  in the binding panel).
- **AMS unit auto-detection** — the bundled bambi instance hard-
  codes 1 AMS lite + 4 slots. Switching to a different AMS topology
  needs the printer-instance editor (post-MVP).
- **Mock-broker automated smoke** — too much apparatus for too
  little signal; PR-7a-2..-6 unit tests cover protocol shape
  against captured fixtures, and the point of this gate is real
  hardware.

## Diagnostics

When something doesn't work:

- **"Test connection" hangs** → check you're on the same subnet
  as the printer, and that the printer's LAN-only mode is enabled.
  MQTT-over-TLS won't reach the printer through most NAT setups.
- **`Invalid certificate` errors in the trace log** → the BBL CA
  may have expired (current expiry: 2032-04-01). Bambu rotates the
  CA periodically; if you hit this before that date, check whether
  Bambu has pushed a CA update via printer firmware.
- **MQTT stream trace logs**:
  ```
  RUST_LOG=n3o_slic3r_lib::core::driver::bambu=trace npm run tauri dev
  ```
  Dumps every status push + every command publish to stderr.
- **AMS strip empty when spools are loaded** → the AMS lite reports
  state via the same `mc_print/report` topic the printer status
  uses; if it's empty, the printer hasn't published an AMS payload
  yet (can take 10–30s after first connect). Reconnect or wait.
- **Send succeeds but printer doesn't start** → check the printer
  LCD for an error code (filament runout, door open, etc.).
  Firmware error states aren't surfaced in the panel yet — Phase
  7c will close that gap.
