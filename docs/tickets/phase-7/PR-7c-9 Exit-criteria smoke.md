# PR-7c-9 — Phase 7 exit-criteria smoke + walkthrough doc

Status: ❌ open.

**Scope.** Mechanizes Execution Plan §9's exit criteria as a
real-hardware walkthrough doc + a minimal automated test for
the binding / mismatch / sync-on-send paths that don't need a
live printer.

**Acceptance criteria.**

- **Automated half** —
  `src-tauri/tests/phase7_smoke.rs`:
  1. Build a project with 2 plates: plate 1 bound to A1 mini,
     plate 2 bound to U1.
  2. Synthesize FilamentState for both printers (different
     loadouts per printer).
  3. Auto-bind both plates. Assert each gets a printer-
     appropriate binding.
  4. Detect mismatches. Assert: 0 mismatches on a properly-
     bound plate; family mismatch detected when we artificially
     swap a slot's reported filament.
  5. Sync-on-send: build a `SlicedPlate` for plate 1 (Bambu)
     + a gcode body for plate 2 (U1). Inject bindings.
     Assert `.gcode.3mf` JSON contains the Bambu binding
     metadata; assert U1 gcode header contains the
     `filament_settings_id_N` comments.

- **Manual half** — `docs/phase-7-smoke.md`:

  1. Pre-flight: both printers powered on + on LAN. Each
     loaded with 4 (or any reasonable count) of distinct
     filaments.

  2. **Multi-printer monitoring**:
     - Create plates 1 + 2 bound to A1 mini + U1.
     - Confirm both PrinterPanels show "Connected" + live
       loadouts.
     - Idle both printers for 30s. Confirm status updates
       continue (no event-firehose throttling broke).

  3. **Filament sync live**:
     - Walk to A1 mini, swap a spool to a different filament.
     - Within 5 seconds, the app's AMS strip updates to show
       the new filament.

  4. **Manual override + badge**:
     - On a U1 slot, set a manual override via
       FilamentStatePanel. Confirm the override badge
       appears.
     - Confirm the override survives a project save + reload.

  5. **Mismatch detection**:
     - Load a 4-color print on plate 1. Bind model material 1
       → A1 mini slot 1.
     - On the printer, change slot 1's filament to PETG
       (printer reports PETG).
     - The MaterialBindingPanel shows a family-mismatch
       warning + the Slice button is disabled with a tooltip.
     - Fix by swapping back / accepting the mismatch in
       settings.

  6. **End-to-end multi-color print on U1**:
     - Plate 2 (U1-bound) loaded with a 4-color model.
     - Auto-bind. Confirm 4 bindings made automatically by
       family-match.
     - Slice + send. Watch all 4 toolheads cycle. Print
       completes with correct colors per material.

  7. **Cross-printer rebinding**:
     - Move plate 1 from A1 mini to U1.
     - Picker offers "auto-bind materials for new printer?"
     - Accept. Bindings update.
     - Switch plate back to A1 mini. Original bindings
       restored (per-(plate, printer) persistence).

  8. **Same project to both printers**:
     - Plate 1 (A1 mini) and plate 2 (U1) both have the same
       4-color model.
     - Slice both. Send each to its respective printer.
     - Both print to completion with correct colors.

- **CI runs the automated half**. Real-hardware steps are
  human-driven.

**Effort.** ~1.5 days for walkthrough + automated test
authoring. Execution time on top.

**Dependencies.** ALL of PR-7a + PR-7b + PR-7c-1..-8.

**Out of scope.**

- Performance gates (status update latency, slicing
  throughput) — Phase 9 if we discover an issue.
- Webcam / camera surfaces (mTLS bootstrap on U1 + Bambu
  RTSP) — post-MVP.
