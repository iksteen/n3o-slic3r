# PR-7a-8 — Bambu real-print smoke + walkthrough doc

Status: ✅ done. Walkthrough captured in `docs/phase-7a-smoke.md`; all
four scenarios (single-color, multi-color AMS, external-spool only,
external-spool + AMS mixed) ran to completion on live A1 mini
hardware.

**Scope.** Phase 7a exit gate. Mechanizes the
"slice → send → print" loop on a real A1 mini, with both a
single-color and a 4-color AMS print, captured as a manual
walkthrough (live hardware required, no automated half — the
driver-side unit tests in PR-7a-2..-6 cover protocol parsing
in CI).

**Acceptance criteria.**

- **Walkthrough doc** `docs/phase-7a-smoke.md`:

  1. Pre-flight: A1 mini powered on, on the same LAN as the
     dev machine, AMS lite mounted with at least 1 spool.
  2. Launch the app. Bind plate 1 to the A1 mini via the
     MaterialBindingPanel.
  3. `PrinterCredentialsDialog` appears. Enter host (printer
     IP or `printer.local`), access code (from printer LCD →
     Settings → Network), serial (from printer device label or
     LCD). Click "Test connection." Wait for green check.
  4. PrinterPanel mounts; verify Connected status, current
     temps reasonable (≤30°C on a cold printer), AMS strip
     shows the loaded spool(s).
  5. Load `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`
     onto plate 1. Bind model material 1 to slot 1 (whichever
     AMS slot has filament). Slice.
  6. After slice finishes, click Send. The panel shows
     "uploading…" briefly, then state transitions to RUNNING.
  7. Watch the panel update layer-by-layer for the first 3
     layers. Verify temps climb toward target. Cancel via the
     Stop button. Confirm dialog. State transitions to FAILED
     (cancel is reported as failed by Bambu firmware).
  8. Re-slice + re-send. Let it print to completion.
     Verify state ends at FINISH and the AMS strip's active-
     slot indicator goes away.

- **Multi-color sub-walkthrough** (separate section in the
  doc):
  - Load `examples/spike3/fourcolor.3mf` onto plate 2.
  - Bind plate 2 to the A1 mini. AMS lite needs ≥2 spools
    loaded; bind model material 1 → slot 1, material 2 → slot
    2, etc.
  - Slice. Confirm a tool-change emission in the preview
    (PR-6-5's Tool color mode lights up multiple colors).
  - Send. Watch for AMS swap mid-print (the active-slot ring
    moves between slots in the AMS strip).
  - Print to completion.

- **External-spool sub-walkthrough** (separate section in the
  doc; required to close this ticket):

  - **External-spool only**: bind a model material to the A1
    mini's `Ext` (external/direct-feed) slot via the binding
    panel. Slice + send. Verify:
    - `ams_bindings` in the wrapped `.gcode.3mf` is empty for
      that material (Ext routes via the `ams_mapping` `-1`
      sentinel, not via AMS slot — see commit `9458e56`
      regression test for the encoder).
    - `use_ams = false` in the project_file MQTT publish.
    - The printer doesn't try to load from AMS; pulls from the
      external spool directly. Print completes.

  - **External spool + 1 AMS slot** (2-material print, if
    libslic3r emits a tower for it): bind M1 → Ext, M2 → an
    AMS slot. Slice + send. Verify:
    - `ams_bindings` contains only M2's entry (M1 is omitted).
    - `use_ams = true` (at least one material is AMS-fed).
    - During print, the printer alternates between the external
      spool and the AMS slot for the two materials. Manual
      loading/unloading of the external spool may be required if
      the firmware doesn't auto-handle the source switch — note
      observed behavior in the doc either way.
  - These two cases pin behavior the off-by-one fix
    (`9458e56`) restored — material on Ext should NOT publish as
    AMS slot 1 (the old bug); the encoder now omits it correctly.

- **What the doc explicitly does NOT cover** (called out so
  later phases know):
  - Pause + resume from the app — separate manual case but
    not gated here (PR-7a-6 unit tests cover the command
    publish).
  - Per-slot filament family mismatch detection — Phase 7c.
  - Send-time AMS binding emission into the `.gcode.3mf` —
    Phase 7c-7. Until that lands, the printer uses the AMS
    bindings the slicer baked in (which match whatever the
    user manually set in the binding panel).

- **Diagnostic affordances** documented inline:
  - "If Test connection hangs: check that you're on the same
    subnet as the printer, and that the printer's LAN-only
    mode is enabled."
  - "If you see `Invalid certificate` errors in the trace log,
    the BBL CA may have expired (current expiry 2032-04-01)."
  - "Trace logs of the MQTT stream: `RUST_LOG=n3o_slic3r::core::driver::bambu=trace npm run tauri dev`."

- **No automated test in CI** — driver protocol parsing is
  unit-tested in PR-7a-2..-6 with captured fixtures; the
  live-printer step can only be a human walkthrough.

**Effort.** ~1 day. Walkthrough authoring is fast; the bulk
of the time is running the walkthrough end-to-end and
debugging whatever surfaces (firmware quirks, AMS spool
hiccups, etc.) that the per-ticket development sessions
didn't catch.

**Dependencies.** All of PR-7a-1..-7.

**Out of scope.**

- Automated smoke against a mock MQTT broker — too much
  apparatus for too little signal; the per-ticket fixture
  tests already cover protocol shape. The point of this gate
  is real hardware.
- Performance gating (status update latency, etc.) — Phase 9
  if we discover an issue.
