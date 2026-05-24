# PR-7b-9 — U1 real-print smoke set

Status: ❌ open.

**Scope.** Phase 7b exit gate. Four real prints on live U1
hardware: single-material, 2-material, 4-material color test,
and a tool-change stress test. Walkthrough doc analogous to
PR-7a-8.

**Acceptance criteria.**

- **Walkthrough doc** `docs/phase-7b-smoke.md`:

  1. Pre-flight: U1 powered on, on the LAN, 4 toolheads loaded
     with the test filaments described below.

  2. **Connection**: bind plate 1 to the U1 via the
     MaterialBindingPanel. Credentials dialog appears →
     enter host + port (default 80). Test connection. Confirm
     `U1ToolheadStrip` lights up with the 4 loaded filaments.

  3. **Single-material print** (Benchy or 20mmbox):
     - Slot 1: PLA red, slots 2/3/4: any.
     - Bind model material 1 → slot 1.
     - Slice + send. Confirm state → RUNNING, tool 0 stays
       mounted throughout, print completes.

  4. **2-material print**:
     - Load `examples/spike3/fourcolor.3mf`, bind model
       materials 1-2 to slots 1-2 (ignore 3-4 in the binding).
     - Slice. Confirm preview's Tool color mode shows 2
       colors only.
     - Send. Watch `U1ToolheadStrip` — the active-toolhead
       ring should move between slots 1 and 2 mid-print.
     - Print to completion.

  5. **4-material print**:
     - Load `fourcolor.3mf`, bind all 4 model materials.
     - Slice. Confirm 4 colors in the preview's Tool mode.
     - Send. Watch all 4 toolheads cycle.
     - Print to completion.

  6. **Tool-change stress test**:
     - Load a model deliberately designed for many tool
       changes (e.g. a 2-color horizontal-banded test cube;
       check in as `tests/fixtures/u1-stress/bands.3mf`).
     - Bind 2 materials. Slice + send.
     - Watch for: tool-change reliability (no skipped
       changes), purge dump sanity (no excessive blobs),
       no firmware error states.
     - Print to completion.

  7. **Pause + resume**:
     - During any of the above prints, click Pause. Confirm
       state → PAUSED, head moves to safe position.
     - Click Resume. Confirm state → RUNNING, print continues
       where it left off.

  8. **Stop**:
     - During any print, click Stop. Confirmation dialog.
     - State → FAILED, head parks, bed cools.

- **Diagnostic affordances**:
  - "If subscribe fails: visit `http://<host>/printer/info`
    in a browser. If it 404s, Moonraker isn't running."
  - "Trace logs: `RUST_LOG=n3o_slic3r::core::driver::u1=trace`."
  - "If tool changes drop filament: check the cascade's
     change_filament_gcode against Snapmaker Orca's
     reference, may need a flush_distance tweak."

- **What this gate does NOT cover** (called out):
  - Filament sync (matching loaded filament to binding) —
    Phase 7c.
  - Mismatch detection — Phase 7c.
  - Build plate auto-detection (if the U1 doesn't report it,
    manual selection only).

**Effort.** ~1 day for walkthrough authoring + execution. Real-
print debugging time is on top.

**Dependencies.** All of PR-7b-1..-8.

**Out of scope.**

- Automated smoke against a mock Moonraker — too much apparatus;
  unit tests cover protocol shape.
- Mixed-nozzle real print (Spike 2's claim) — post-MVP per the
  cascade ticket scope.
