# PR-2-11 — Perf stress test + Three.js vs wgpu pivot decision

Status: 📅 deferred (post-Phase-2 evaluation, not a Phase 2 gate).

**Scope.** *Originally framed as a Phase 2 deliverable that decides
between shipping Three.js or pivoting to wgpu against measured FPS
budgets.* Post-Phase-1 review reframed this as a **post-MVP
evaluation**, not a Phase 2 exit-gate:

- The schedule is **comfortably ahead** — burning ~3 days on a
  pivot decision in Phase 2 is premature.
- The dev rig is a high-end Nvidia 5070 Ti — the original
  "integrated GPU laptop" framing doesn't match the realistic test
  environment. Real perf-on-modest-hardware evaluation needs to
  happen on actual modest hardware, which is a Phase 9 (release
  prep) concern.
- The AD-8 architectural separation (PR-2-1 / PR-2-2 state-vs-
  renderer) is the load-bearing guarantee that lets us swap
  renderers later; as long as that's intact, the pivot remains
  cheap whenever we need it.

**New plan: ship Three.js, measure later.**

- Phase 2 ships with **Three.js + WebGL**. No FPS gate. No formal
  budget assertion. Visual smoke-test sufficiency is "looks
  smooth on the dev rig" for the duration of Phase 2 work.
- The **5 ms p99 state-ops budget on a 1000-object scene** stays
  — that's a Rust-side regression gate independent of the
  renderer choice, similar to PR-1-10's resolver perf gates.
  Implementer ships it as `src-tauri/tests/scene_state_perf.rs`
  with the same Instant-based timing pattern as PR-1-10.
- The renderer-side FPS measurement lands as a **Phase 9 release-
  prep task**: actual-modest-hardware testing, FPS measurement
  on the lowest-spec laptop on the test rig, decision to ship
  Three.js or pivot.

**Reduced Phase 2 deliverables.**

- `src-tauri/tests/scene_state_perf.rs` — Rust integration perf
  test. Builds a 1000-object scene; measures + asserts:
  - Single-object transform (translate, rotate, scale): mean +
    p99 < 5 ms.
  - Selection toggle of 100 objects: mean + p99 < 5 ms.
  - Full `scene_snapshot()` serialization: < 50 ms (reconnect
    path, less frequent than per-interaction ops).

**Deferred to Phase 9 (release prep).**

- 20M-triangle FPS measurement on a target modest-hardware rig.
- `docs/phase-2-renderer-decision.md` pivot decision doc.
- Three.js ↔ wgpu pivot scoping if needed.

**Effort.** ~1 day for the Rust-side perf test (was ~3 days
including the Three.js measurement + decision doc).

**Dependencies.** PR-2-1 / PR-2-2 / PR-2-5 (scene state + ops the
perf test exercises).

**Out of scope.** Everything Phase-9-shaped above.
