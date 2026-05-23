# PR-3-12 — Phase 3 exit-criteria smoke

Status: ❌ open.

**Scope.** End-to-end smoke that exercises Phase 3's exit criteria
as a single repeatable test. Mirrors PR-0-5 / PR-1-11 / PR-2-12:
documented procedure + automated half + CI hook.

The Phase 3 exit criteria from Execution Plan §5 are the **independent
oracle** for the slice loop:

> Verification approach: parse, re-serialize, byte-diff equals zero
> on G-code; structural-diff equals zero on 3MF. This is the
> project's independent oracle — no external slicer needed.

**Acceptance criteria.**

- `docs/phase-3-smoke.md` documents the procedure:
  1. `cargo test --workspace` — all PR-3-* tests pass, including
     the parser perf gate from PR-3-6 (50 MB G-code < 3 s).
  2. `npm test` — frontend slice-reducer test passes (PR-3-4).
  3. `npm run tauri dev` — viewport opens, click **Slice** on a
     single-cube scene, watch progress events stream, see
     summary panel populate with time + filament numbers.
  4. The resulting G-code parses cleanly via PR-3-6 with zero
     `ParseError` results and round-trips byte-equal via PR-3-7
     (verified by the automated smoke; documented for humans).
  5. Load `examples/spike3/fourcolor.3mf` via PR-2-4's reader,
     write it back via PR-3-9's writer, reload the result — the
     reloaded `Project3mf` is structurally equivalent to the
     original.

- `src-tauri/tests/phase3_smoke.rs` (automated half — 6+ steps):
  - Step 1: slice a tiny cube scene end-to-end through PR-3-2's
    orchestrator. Assert it completes, produces a G-code file,
    and surfaces a `PlateSummary` with non-zero time + filament.
  - Step 2: parse the slice output via PR-3-6. Assert zero
    `ParseError` results.
  - Step 3: serialize the parsed output via PR-3-7. Assert byte
    equality with the slice output.
  - Step 4: parse `examples/spike1/*.gcode` (50 MB realistic
    fixture from PR-0.5-1). Assert parse completes in < 3 s.
  - Step 5: round-trip `examples/spike3/fourcolor.3mf` through
    the reader + PR-3-9's writer. Assert structural equivalence.
  - Step 6: round-trip a synthesized `SlicedProjectInput` through
    PR-3-10's writer (skipped gracefully if PR-3-10 was cut to
    minimum-viable scope).

- CI: the automated smoke runs in the existing
  `cargo test --workspace` step; no new CI jobs.

- Human-driven half: walk through the `npm run tauri dev` steps
  in the doc — slice from the UI, watch the progress bar move,
  see the summary card render.

**Effort.** ~1 day.

**Dependencies.** Every other Phase 3 ticket. This is the last one
to land.

**Out of scope.** Phase 4 (Settings UI) — Phase 3's smoke uses the
bundled A1-mini cascade defaults; settings overrides happen in the
next phase. End-to-end real-print validation — Phase 7a's
responsibility. Tool-change minimization assertion — that's PR-3-11,
which may or may not have landed by smoke time; the smoke logs the
tool-change count but doesn't assert a budget.

**The smoke is the project's gate.** If a future change breaks
parser → serializer byte-equivalence, the smoke fails; the diff
points at the model field that lost information; the model expands
to preserve it. That's how the independent oracle stays load-bearing
through Phase 4+.
