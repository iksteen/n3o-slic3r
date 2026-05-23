# PR-5-4 — Per-plate printer assignment + cascade re-resolution

Status: ❌ open.

**Scope.** When the user assigns a different printer to a
plate, the cascade re-resolves for that plate's context (new
printer = different `printer.model`, `printer.slot_count`,
`printer.toolheads.len()`, supported build plates), and
incompatible settings surface as warnings (not silent
corrections).

Owns FR-MP-2 + FR-MP-3.

**Acceptance criteria.**

- New Tauri command:
  ```rust
  scene_set_plate_printer(
      plate_id: PlateId,
      printer_identity: String,
      build_plate_identity: String,
  ) -> Result<PrinterChangeReport, String>
  ```
  Validates that the printer exists in the profile registry,
  updates the plate's `PrinterBinding`, recomputes the
  `BedMesh` for the new printer, returns a typed report
  documenting which settings are now invalid for the new
  printer (e.g. wall_filament=3 on a printer with slot_count=2).

- `PrinterChangeReport`:
  ```rust
  pub struct PrinterChangeReport {
      pub plate_id: PlateId,
      pub previous_printer: String,
      pub new_printer: String,
      /// Settings whose value is now out of range / not
      /// applicable. Surfaced as non-blocking warnings.
      pub incompatible: Vec<IncompatibleSetting>,
      /// Settings auto-clamped to the new printer's range
      /// (e.g. extruder index reduced).
      pub clamped: Vec<ClampedSetting>,
  }
  ```

- Emits `scene:printer_changed { plate_id, report }` so the
  frontend can render warning toasts inline + flag affected
  rows in the settings panel.

- Frontend integration: PR-4-5's `BuildPlateSelector` already
  reads `printer.supported_build_plates`; extend the config
  strip with a `config-chip-printer` button that opens a
  printer-picker menu (lift from mockup's
  `.printer-picker-menu` at `SettingsPanel.jsx:546-557`).
  Selection calls `scene_set_plate_printer` and surfaces the
  warning report via toast.

- Cascade re-resolution: the panel's `useCascadeResolve`
  (PR-4-4) already rebuilds the `ContextJson` on every
  resolve call; the printer-change event just triggers a
  re-resolve naturally.

- Tests:
  - 3-plate fixture: change plate 2 from A1 mini → U1;
    plate 1 + plate 3 cascades unchanged; plate 2's
    `wall_filament` cascade resolves against U1's
    slot_count.
  - Incompatible-setting report: project override sets
    `wall_filament = 3` while bound to a slot_count=2
    printer → re-bind to slot_count=4 → incompatible list
    empties; re-bind back → reappears.
  - Build-plate change without printer change: re-resolves
    BedTempPerPlate dimensional expansion correctly.

**Effort.** ~2 days. The cascade re-resolution is already
wired (PR-4-4); the work is the printer-picker menu UI,
the warning report shape, and the toast rendering.

**Dependencies.** PR-5-2 (per-plate SceneState), PR-5-1
(`PrinterBinding`), Phase 4's SettingsPanel resolve loop.

**Out of scope.** Profile authoring UI (Phase 9). Printer
discovery via mDNS / connection polling (Phase 7).

**Cut candidate.** The warning toast for incompatible
settings (~half day) — the SettingsPanel rows would still
re-render with the new resolved values; the user just
wouldn't get a proactive notification. Cut if shipping
date pressure hits.

**Design reference.** `docs/design/SettingsPanel.jsx`'s
`.printer-picker-menu` block (around line 546) is the
canonical menu pattern: list of `.ptpm-item` rows, each
with `.ptpm-name` + `.ptpm-detail`, active row marked.
