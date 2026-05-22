# PR-2-11 — Perf stress test + Three.js vs wgpu pivot decision

Status: ❌ open.

**Scope.** Validate Phase 2's two performance budgets against real
workloads. If we hit the targets, ship Three.js. If we don't,
schedule the pivot to wgpu and budget the work into Phase 3 or
Phase 4 (whichever has slack).

This is the **decision point** PRD §10 calls out as a risk — Three.js
on integrated GPUs at 20M triangles is uncertain; the AD-8
separation exists precisely so the pivot doesn't blow the schedule.

**Two budgets to validate:**

1. **20M-triangle scene at ≥30 fps on integrated-GPU laptop.**
   Geometry render-side perf — Three.js draw calls + GPU
   throughput. Test fixture: 100 copies of a 200k-triangle model
   (or 1000 copies of a 20k-triangle model) on a single plate.

2. **5 ms p99 state ops on a 1000-object scene.** Rust scene-state
   side. Selection / transform / diff computation must stay under
   the budget so the event loop never blocks the renderer's frame
   pacing.

**Acceptance criteria.**

- **`src/viewport/__test__/perf_20m_triangles.tsx`** — interactive
  perf harness loading a programmatically-generated 20M-triangle
  scene; reports FPS during a scripted orbit. Run with `npm run
  tauri dev` + manual inspection (or via a headless puppeteer
  setup if Phase 4 ships one). Records FPS over 30 seconds; the
  budget assertion is ≥30 fps p10 (the slowest 10% of frames).

- **`src-tauri/tests/scene_state_perf.rs`** — Rust integration
  perf test (same pattern as PR-1-10's `cascade_perf`). Builds a
  1000-object scene; measures:
  - Single-object transform (translate, rotate, scale): mean +
    p99 < 5 ms.
  - Selection toggle of 100 objects: mean + p99 < 5 ms.
  - Full `scene_snapshot()` serialization: < 50 ms (this is the
    reconnect path, less frequent than per-interaction ops).

- **Pivot decision document** at `docs/phase-2-renderer-decision.md`:
  - Records measured FPS at 20M triangles and the test
    configuration.
  - Reports the Rust-side perf numbers.
  - Recommends **Three.js (ship)** or **wgpu (pivot)** with a
    one-page rationale.
  - If pivot: scopes the work (new renderer crate + GPU pipeline
    + scene-event reconciler) and slots it into Phase 3 / Phase 4
    with a target landing date.

- The 5 ms scene-state budget must pass regardless of the
  renderer-side outcome — the pivot doesn't help if the state
  side is the bottleneck.

**Effort.** ~3 days. The 20M-triangle fixture + perf measurement is
~1 day; the Rust-side test is ~1 day; analysis + decision-doc
authoring is ~1 day.

**Dependencies.** PR-2-9 (renderer to test). The Rust-side test can
run earlier if PR-2-1/PR-2-2/PR-2-5 are done — useful to know the
budget passes before investing more renderer time.

**Out of scope.** Actual pivot to wgpu — if the decision is "ship
Three.js," none. If the decision is "pivot," that's a separate
work item with its own ticket (PR-3-X or PR-4-X, TBD by scheduling).
GPU profiling beyond FPS averages — Phase 9 if release-perf
regressions surface.

**Notes.** Don't optimize prematurely. If Three.js scrapes by at
32 fps on the test rig, ship it. The 30 fps bar is intentionally
forgiving — perfect smoothness isn't the bar; "doesn't feel awful"
is.
