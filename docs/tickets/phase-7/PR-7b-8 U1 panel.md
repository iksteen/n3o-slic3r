# PR-7b-8 — Frontend U1 state panel

Status: ✅ done. Component in `src/driver/U1ToolheadStrip.tsx`; wired into `PrinterPanel.tsx` next to the existing `BambuAmsStrip` branch; projection tests in `src/driver/__test__/U1ToolheadStrip.test.ts`.

## Deviations from the acceptance criteria

- **`PrinterCredentialsDialog` U1 branch already shipped in PR-7b-7**
  ("kind-aware credentials dialog + U1 wire-up"). Nothing to add
  here; the dialog handles U1 (host + port, no access code/serial)
  and the "Test connection" path through `driver_register` +
  `driver_connect` was already wired.
- **Layer-count placeholder already correct.** `formatJobLine`
  in `PrinterPanel.tsx` only emits the `L N/M` segment when both
  `current_layer` and `total_layers` are non-null — the U1 path
  already drops the segment cleanly without `Layer ?/?`. No edit
  needed.
- **`TempsLine` reduced to bed-only for U1 contexts** (small
  detour from the ticket text). U1 reports 4 independent nozzles;
  rendering `nozzles[0]` in TempsLine *plus* per-cell temps in the
  strip would double the T0 reading. TempsLine now takes a
  `kind: DriverKind` prop and renders `B <bed>` only when
  `kind === "U1"`; Bambu's behavior is unchanged.
- **Shared color helper extracted** to `src/driver/colorUtils.ts`.
  Both `BambuAmsStrip` and `U1ToolheadStrip` need the same
  RGBA-hex → CSS-color normalization (alpha-`FF` stripped); a
  sibling file is cleaner than re-exporting from one strip
  through the other.
- **Visual divergence from `BambuAmsStrip`** is intentional. Bambu
  is a chip row (single hotend, AMS picks one filament at a time
  — chip + tooltip says enough). U1 is a row of vertical mini-cards
  (4 independent toolheads, each with permanent filament + own
  temp — chip + label + temp readout per cell is the natural
  shape). They share the active-ring + dashed-empty +
  color-normalization conventions so the panels still read as
  siblings.

**Scope.** PrinterPanel rendering for U1 contexts. Reuses the
component scaffold from PR-7a-7 with U1-specific extras: 4-tool
state strip (loaded filament + active toolhead ring + per-tool
temps).

**Acceptance criteria.**

- **Extend `PrinterPanel.tsx`** from PR-7a-7:
  - Branch on `driverKind` for the extra-state rendering.
  - For `Bambu` → render `BambuAmsStrip`.
  - For `U1` → render `U1ToolheadStrip` (new).

- **`U1ToolheadStrip.tsx`**:
  - 4 cells, one per toolhead.
  - Per cell:
    - Color chip from `U1Filament.color` (RGBA hex).
    - Material type from `U1Filament.material_type`
      (truncated to 6 chars).
    - Temp readout: `T<n> TTT/SET`.
    - Active-toolhead ring around the mounted slot.
  - Empty toolhead (no `print_task_config` filament reported):
    dashed outline + "—".

- **Layer-count placeholder**: when
  `JobProgress.total_layers` is `None` (the U1 doesn't expose
  it natively — see PR-7b-3 known gap), the job-line shows
  `<file_name> — XX% — ETA HH:MM:SS` without the
  `Layer N/M` segment. Don't render "Layer ?/?".

- **`PrinterCredentialsDialog`** branch:
  - For U1: two inputs — host + port (default 80). No access
    code, no serial. Serial probed automatically.
  - "Test connection" button → `driver_register` +
    `driver_connect` (probe runs as part of connect).

- Tests:
  - `U1ToolheadStrip.test.tsx` — render with 2/4 loaded, all
    empty, active toolhead variants.
  - `PrinterCredentialsDialog.test.tsx` — extend to cover the
    U1 branch.

**Effort.** ~1 day. Mostly the toolhead strip component +
extending the credentials dialog.

**Dependencies.** PR-7a-7 (panel scaffold), PR-7b-2..-6 (driver
surface).

**Out of scope.**

- Webcam tile — the U1 has a camera but the mTLS pairing flow
  required is out of MVP scope.
- Per-toolhead probe / level-bed affordances — printer-side
  function, not exposed via Moonraker subscribe set.
