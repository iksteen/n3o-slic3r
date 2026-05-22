# PR-2-12 — Phase 2 exit-criteria smoke

Status: ❌ open.

**Scope.** End-to-end smoke procedure that exercises Phase 2's
exit criteria as a single repeatable test. Mirrors PR-0-5 and
PR-1-11 — documented procedure + (where possible) a CLI driver +
CI hook.

**Acceptance criteria.**

- `docs/phase-2-smoke.md` documents the procedure:

  1. `cargo test --workspace --release` — all PR-2-* tests pass,
     including scene-state command/event contract tests + 3MF
     loader round-trip + transform op composition tests + perf
     gates from PR-2-11.
  2. `npm run tauri dev` — app window opens, viewport renders the
     bed mesh + grid + exclusion zones for the active printer.
  3. Drop a 50 MB STL onto the viewport (or use a file-picker
     command); the mesh loads in < 3 s and appears at origin.
  4. Drag-select the object, drag with the translate gizmo —
     position updates in real time.
  5. Open a Bambu-Studio-authored `.3mf` (e.g.
     `external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf`)
     — all per-part extruder assignments visible in object metadata.
  6. Click the calibration cube in the object library — appears at
     plate origin without re-loading the file.
  7. Click `Frame All` — camera frames every visible object.
  8. Restart the app; on reconnect the scene snapshot rebuilds the
     viewport correctly (state survives renderer disconnect).
  9. PR-2-11's perf decision is recorded in
     `docs/phase-2-renderer-decision.md`.

- CI hook (extend `.github/workflows/build.yml`):
  - The Rust scene-state perf test
    (`src-tauri/tests/scene_state_perf.rs`) runs in the existing
    `cargo test --workspace --release` step.
  - The Three.js 20M-triangle FPS test is local-only for MVP —
    add a `puppeteer` / `headless WebGL` CI run later if Phase 9
    release-prep wants automated FPS regression gating.

- The smoke procedure runs cleanly from a clean checkout. Any
  divergence from the documented expected output is a bug or a
  documentation update.

**Effort.** ~1 day.

**Dependencies.** All other Phase 2 tickets complete.

**Out of scope.** GUI screenshots — Phase 4 ships proper UI
testing infra. Visual-regression testing — not in MVP scope.
Multi-printer / multi-plate smoke — Phase 5.
