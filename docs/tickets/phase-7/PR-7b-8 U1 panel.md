# PR-7b-8 — Frontend U1 state panel

Status: ❌ open.

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
