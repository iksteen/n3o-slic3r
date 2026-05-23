# PR-2-12 — Phase 2 exit-criteria smoke

Status: ✅ shipped — automated half lives in `tests/phase2_smoke.rs` + `npm test`; human-driven viewport half in `docs/phase-2-smoke.md`.

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
  3. Drop the 47 MB Stormtrooper Helmet fixture
     (`examples/perf-fixture/stormtrooper-helmet.3mf`, staged at
     PR-2-3 implementation time per its NOTICE) onto the viewport
     — mesh loads in < 3 s and appears at origin. Same fixture
     re-saved as STL round-trips through PR-2-3's STL loader within
     the same budget.
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
  9. PR-2-11's Rust-side scene-state perf gates pass via
     `cargo test --workspace --release`. The renderer-side FPS
     measurement + Three.js/wgpu pivot decision is **deferred to
     Phase 9 release prep** (per the PR-2-11 reframe — dev rig is
     a high-end GPU; real perf-on-modest-hardware lives with
     release testing).

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
