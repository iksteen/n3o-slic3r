# PR-6-17 — Phase 6 exit-criteria smoke + docs

Status: ❌ open.

**Scope.** Mechanize the Execution Plan §8 exit criteria
as a runnable smoke test + a manual walkthrough doc.
Mirrors the Phase 5 smoke pattern (`phase5_smoke.rs` +
`docs/phase-5-smoke.md`).

**Acceptance criteria.**

- **Automated half** — `src-tauri/tests/phase6_smoke.rs`:
  1. **Slice from scene:** build a multi-plate project
     (A1 mini + U1, one cube per plate), call
     `slice_active_plate(PlateId(1))` via the orchestrator
     entry. Wait for `SliceEvent::JobFinished`. Assert the
     output `.gcode` exists.
  2. **Preview load:** invoke `preview_load(output_path)`.
     Assert: returns a valid handle, `layer_count > 0`,
     `bounding_box.max[2] > 0`.
  3. **Color modes round-trip:** for each ColorMode (5
     variants), invoke `preview_buffers(handle, mode,
     Default)`. Assert the returned binary buffer length
     is non-zero and consistent across modes.
  4. **Stats consistency:** invoke
     `preview_layer_stats(handle)`. Assert
     `layer_count == response.layer_count`,
     `sum(layer.duration_seconds) ≈
     job_stats.total_duration_seconds` (within 5%).
  5. **Foreign-slicer compat:** load three checked-in
     fixtures (Orca / Cura / Prusa, sourced per the index's
     open question). For each, assert
     `preview_load` succeeds, header metadata extracts
     correctly (slicer-of-origin matches), at least one
     `FeatureType` variant other than `Unknown` appears.
  6. **`.gcode.3mf` round-trip:** invoke
     `preview_load_gcode_3mf` on a sliced `.gcode.3mf`
     (PR-3-10 writer produces one in-test). Assert
     plate metadata extracts correctly + thumbnail bytes
     are present.
  7. **Cleanup:** invoke `preview_drop(handle)` for each
     loaded preview; assert subsequent lookups error.

- **Manual half** — `docs/phase-6-smoke.md`:
  - Mirror `docs/phase-5-smoke.md` structure.
  - Walkthrough:
    1. Launch the app, build a 3-plate scene (PR-5-12's
       fixture).
    2. Slice plate 1 via the topbar Slice button. Confirm
       auto-switch to preview mode.
    3. Step through all 5 color modes; spot-check that the
       legend updates + the geometry recolors.
    4. Scrub the layer slider in single, up-to-N, and
       range modes. Confirm 60fps subjectively.
    5. Toggle travels + retractions on; confirm visibility
       changes.
    6. Hover over a segment; confirm tooltip shows source
       gcode line + position + feature.
    7. Drag-drop a foreign-slicer `.gcode` file from disk.
       Confirm it loads + renders.
    8. Drag-drop a `.gcode.3mf` (export one from Bambu
       Studio if available). Confirm it loads + the
       thumbnail surfaces in the stats panel.
    9. Switch plate tabs; confirm each plate's last-sliced
       G-code mounts (or empty state if not yet sliced).

- **Foreign-slicer fixtures:** new directory
  `src-tauri/tests/fixtures/foreign-gcode/`:
  - `cube_orca.gcode` (~50KB)
  - `cube_cura.gcode` (~50KB)
  - `cube_prusa.gcode` (~50KB)
  - One small cube sliced in each. Attribution in
    `NOTICE.md` if any of the source files require it
    (they shouldn't — these are user-authored outputs).

- **CI integration:** `cargo test --test phase6_smoke` runs
  on every PR. Foreign-slicer compat is part of the
  default suite (small fixtures, sub-second). The
  `slice-from-scene → preview-load` half pulls in the FFI
  + libslic3r, so its runtime is similar to
  `phase3_smoke` and `phase5_smoke`.

- **Test runtime budget:** the full smoke completes in <30s
  on CI. Anything slower needs investigation.

**Effort.** ~1.5 days. Smoke wiring is fast; the
manual-walkthrough doc + foreign-slicer fixture sourcing
are the bulk.

**Dependencies.** **All of PR-6-1 through PR-6-16.** This
ticket is necessarily last.

**Out of scope.**

- Performance gates (PR-6-16 owns those).
- Sending sliced G-code to a real printer (Phase 7).
- Cross-platform fixture validation (the foreign-slicer
  fixtures cover linguistic compat; platform compat is
  separate).

**Cut candidate.** Foreign-slicer fixtures → save ~0.5
days. Smoke only verifies own-output preview. **Not
recommended** — the foreign-slicer compat claim is
explicit in the exit criteria.
