# Phase 2 exit-criteria smoke

Walks the project's Phase 2 deliverables end-to-end on a clean
checkout. Mirrors `docs/phase-0-smoke.md` and `docs/phase-1-smoke.md`
— half automated (Rust + frontend tests), half human-driven (the
viewport, which needs a real GUI session).

## Automated half — runs in CI

```
$ cargo test --workspace    # CI uses debug profile; see note below
$ npm test
```

CI runs the suite in **debug** mode after the GitHub Actions runner
OOM-killed the release-mode compile on `zvariant v5.11.0` (run
26313140866). Perf-budget tests still pass with > 10× headroom in
debug — translate p99 ≈ 1 µs vs the 5 ms ceiling, resolver mean ≈
26 µs vs 10 ms, snapshot ≈ 100 µs vs 50 ms. Release-mode local runs
remain the canonical headroom check; CI just regresses against the
budgets, which is what matters.

Expected (from the dev rig — modest-hardware budgets fold into Phase 9
release prep):

| Suite                          | Tests | Notes                                                  |
| ------------------------------ | ----- | ------------------------------------------------------ |
| cascade unit                   |  99   | PR-1-* baseline                                        |
| scene state                    |  39   | PR-2-1 .. PR-2-7                                       |
| scene loaders (stl/obj/3mf)    |  14   | PR-2-3 + PR-2-4                                        |
| primitives + library           |  10   | PR-2-7                                                 |
| arrange                        |   5   | PR-2-8                                                 |
| bed / out-of-bounds            |   5   | PR-2-6                                                 |
| cascade_perf                   |   3   | PR-1-10 budgets                                        |
| scene_state_perf               |   6   | PR-2-11 budgets — translate/rotate/scale ≤ 5 ms p99    |
| phase2_smoke                   |   8   | this file                                              |
| frontend vitest                |  13   | PR-2-9 + PR-2-10 mirror + gizmo                        |

Total: **~168 Rust tests + 13 frontend tests, all green**. Any red
result is a regression to fix before tagging the phase.

## Stage the perf fixture (optional)

Step 4 of the procedure below + the `step_4_stormtrooper_loads_under_budget_when_present`
test load a 47 MB 3MF to time the loader on a real-world file. The
file is **not** vendored — its license is CC-BY-NC, so it stays in
the user's local checkout only.

Source: [Stormtrooper Helmet on MakerWorld](https://makerworld.com/)
(author: axp1). Stage it as:

```
examples/perf-fixture/stormtrooper-helmet.3mf
```

When absent, the test prints a skip message and passes. The smoke
procedure's step 4 below will note "skip: fixture missing" instead
of producing a load timing.

## Human-driven half — viewport

```
$ npm run tauri dev
```

The window should open within ~2 s on a warm build.

1. **Bed renders.** The viewport paints a 10 mm grid for the
   currently-active printer (default: Bambu A1 mini) with the bed
   outline at z=0 and any exclusion zones as red AABB wireframes.
   If no bed is visible: check the snapshot path — the renderer
   subscribes to `scene:bed_changed` and `applySnapshot` calls
   `BedChanged` during initial sync.
2. **Add a calibration primitive.** Click the **Debug** button in
   the header to open the Phase 0 debug panel — primitives come in
   through the same path until Phase 4 builds a dedicated library
   panel. From a Rust REPL or via the not-yet-wired UI, invoke
   `scene_object_add_from_primitive` with `Cube` defaults — a 20 mm
   cube appears centered on the plate.
3. **Drag-select.** Click the cube — selection chip in the top-left
   reads "1 selected". Click empty bed — chip disappears.
4. **Stormtrooper load.** From the slice-debug input, point at the
   staged Stormtrooper fixture. Mesh loads in **< 3 s** (verified
   by step 4 of the automated half when the fixture is present).
5. **BBS-flavor 3MF load.** Load
   `examples/spike3/fourcolor.3mf`. Per-part extruder assignments
   (1, 2, 3, 4, 1, 2, 3, 4) ride through the loader (verified by
   step 3 of the automated half).
6. **Translate gizmo.** Click the **T** button in the toolbar. The
   gizmo's translate handles appear around the selected object.
   Drag — the object follows. On mouse-up, the position commits to
   the Rust side via `scene_object_set_transform` and stays put.
7. **Frame All.** Click **Frame all** — camera repositions so every
   visible object fits the viewport.
8. **Snapshot survives restart.** Quit the app; relaunch. The scene
   rebuilds from the persisted state — same objects, same selection,
   same camera (provided Rust persists between launches; the
   in-memory state today does *not*, which is fine — the snapshot
   path is tested via JSON round-trip in step 8 of the automated
   half).

## Out of scope here

- 20M-triangle FPS measurement on a modest GPU. PR-2-11 punts this
  to Phase 9 release prep where actual-modest-hardware testing
  happens.
- Multi-select drag. The PR-2-10 gizmo attaches to the first
  selected object only; per-object delta application is a Phase 4
  follow-up (see task #115 / PR-2-10 ticket footer).
- Calibration tower fixtures. Orca's bundled temperature +
  stringing towers ship as `.drc` (Draco-compressed) which our
  loaders don't decode. PR-2-7 surfaces them as
  `UnsupportedFormat`; sourcing 3MF/STL replacements is task
  #102.

## If a step fails

- Rust tests red: `cargo test --workspace --release -- --nocapture`
  surfaces println output from the perf gates with actual measured
  numbers. Re-run after fixing.
- Vitest red: `npm test -- --reporter=verbose` for per-case
  context.
- Viewport blank: open the DevTools console. The Tauri event
  bridge logs warnings when an event references a mesh the mirror
  doesn't know about; that's the first signal something is out of
  sync.
- OOB warnings stacking up in the toast corner: PR-2-6's check
  fired. Either the object's bounding box really did slip off the
  bed (correct behavior, user-fixable) or the active printer's
  `build_volume` is wrong in the printer profile.
