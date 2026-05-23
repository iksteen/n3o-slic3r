# Phase 5 exit-criteria smoke

Walks the project's Phase 5 deliverables (multi-plate +
multi-printer project model, per-plate cascade re-resolution,
`.3mf` save/load) end-to-end on a clean checkout. Mirrors
`phase-4-smoke.md` — half automated (Rust + frontend tests),
half human-driven (UX flow + actually-driving-a-printer aspects
that need a real GUI session).

Phase 5's structural exit criterion is the **3-plate save +
reload round-trip preserving every authored field**. The
automated half pins that as a Rust integration test. The manual
half walks the same fixture through the live UI so the panel
chrome, the picker, the binding panel, the recovery dialog are
all exercised at once.

## Automated half — runs in CI

```
$ cargo test --workspace
$ npm test
```

What `phase5_smoke.rs` exercises:

1. **Build the 3-plate exit fixture in-memory.** Plate 1 → A1
   mini (`bambu-a1-mini`), plates 2-3 → Snapmaker U1
   (`snapmaker-u1`); one stub triangle mesh per plate; one
   material binding per plate (slot 1 → `Generic PLA`);
   project-tier override `layer_height = 0.12` on plate 2;
   object-tier override `enable_support = 1` on plate 1's
   object; user-tier override `travel_speed = 300`;
   file-metadata `Title` on the project.
2. **Save** to a temp `.3mf` via `write_project` (PR-5-8).
3. **Drop** the in-memory project + **load** the saved file via
   `read_project` (PR-5-8).
4. **Assert per-field equality** on the reloaded project:
   - plate count = 3
   - per-plate printer identity + build-plate identity match
   - project-tier override on plate 2 survived
   - object-tier override on plate 1 survived
   - user-tier override survived
   - one material binding per plate, slot 1 → Generic PLA
   - file metadata survived

What `phase5_smoke.rs` does NOT exercise — covered by the
manual half:

- Slicing each of the 3 plates end-to-end. Each plate needs its
  own `SliceJobInput` + cascade; the per-plate slice loop is
  straightforward to drive manually via the Slice button but
  adds substantial orchestration in Rust for diminishing return.
  `phase3_smoke.rs` already pins the single-plate slice
  contract.
- The recovery dialog flow (PR-5-10) — needs a Tauri window
  context the integration runner doesn't have.
- The settings panel's per-plate cascade re-resolution on
  printer switch — frontend concern, exercised by hand in the
  walkthrough.

## Manual half — 3-plate walkthrough

The flow below is the live-UI counterpart of the automated
fixture. Drive it on a clean checkout (`npm run tauri dev`) and
verify each step matches the expected outcome before moving on.

### 1. Build the three plates

1. Launch the app. Confirm the topbar reads `n3o-slic3r`, the
   plate-tab strip shows one tab (default `Plate 1`), the
   viewport shows the A1 mini bed grid, and the settings panel
   is mounted on the right with `Printer: Bambu A1 mini`.
2. Click `+ New plate` in the tab strip. A second tab appears.
3. Click `+ New plate` again. Three tabs total.

### 2. Bind printers per plate

1. Click `Plate 2`. The plate is unbound — printer chip reads
   `No printer`.
2. Open the printer picker (chip in the settings config strip).
   Pick `Snapmaker U1`. The viewport bed should change to the
   U1's 220×220 build volume + the parking-bay exclusion strip.
3. Click `Plate 3`. Pick `Snapmaker U1` the same way.
4. Click `Plate 1`. Confirm it's still bound to the A1 mini
   (chip + bed unchanged).

### 3. Load geometry

1. On plate 1: load a cube via the file dialog (any small STL —
   `external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl`
   works).
2. On plate 2: load `examples/spike3/fourcolor.3mf`.
3. On plate 3: load any other model (a second cube is fine).

### 4. Author overrides

1. **Plate 2 project-tier**: switch to plate 2. In the settings
   panel, search `layer_height`, type `0.12` in the value cell,
   commit (Enter). The row should pick up the project-tier
   tint (light cyan) and the reset arrow should appear on hover.
2. **Plate 1 object-tier**: switch to plate 1, click the cube
   in the viewport to select it. Switch to the `Object`
   tab in the settings panel. Search `enable_support`, toggle
   it on. Row picks up the object-tier tint (light rose).

### 5. Set material bindings

1. On each plate, the MaterialBindingPanel (under the printer
   chip) should show one row (`M1 → Slot — → filament —` if
   nothing's bound yet).
2. Click `Auto-bind` on each plate. The row fills in
   `M1 → Slot 1 → Generic PLA`.

### 6. Slice the plates

1. Click the Slice button (top-right) on plate 1. Wait for
   `Finished` status — a `.gcode` file appears in `/tmp/`.
2. Same for plate 2.
3. Same for plate 3.

> **Known scope gap**: only A1 mini has a bundled cascade
> today. Slicing plates 2-3 (U1-bound) will resolve against the
> A1 mini cascade as a stand-in; the produced G-code won't be
> send-to-U1 correct. The exit criterion is "produces a
> non-empty `.gcode` file with zero parser errors" — confirmed
> via the slice panel's progress + the on-disk file.

### 7. Save the project

1. Use the Save command (TODO — add a topbar Save button as
   PR-5-9 polish or trigger via the Tauri menu) to save the
   project to `~/n3o-test-3plate.3mf`.

### 8. Reload + verify round-trip

1. Close the app entirely.
2. Re-launch `npm run tauri dev`.
3. The autosave-recovery dialog *may* appear (an entry for
   the in-progress project written by the 30-s autosave
   worker). Recover or Discard as you prefer — this is
   a separate flow from the explicit Save in step 7.
4. Use the Load command to open `~/n3o-test-3plate.3mf`.
5. **Verify:**
   - 3 plates with the same names as before.
   - Plate 1 bound to A1 mini, plates 2-3 to U1.
   - Plate 1 has the cube; plate 2 has fourcolor; plate 3 has
     the second cube.
   - Plate 2's `layer_height` row is tinted project-tier and
     reads `0.12`.
   - Plate 1's cube has its `enable_support` object-tier
     override.
   - Each plate's material binding panel shows
     `M1 → Slot 1 → Generic PLA`.
   - Settings panel rows resolve to the same values as
     before the save (spot-check a handful of cascade-derived
     fields like `nozzle_temperature` per plate).

If any step's assertion fails, the offending field's
round-trip is broken; trace through the `format.rs` write path
+ `read_project` load path.

## Out of scope (deferred)

These were named in the original ticket but cut from MVP /
deferred to later phases:

- **Per-plate cycle count** — `cycle_count` was a
  PlateCycler-plugin-only field; cut to Phase 8 when the
  plugin host arrives.
- **Send-to-printer over LAN** — Phase 7 driver work
  (Bambu LAN protocol + Snapmaker HTTP API). The MVP smoke
  produces `.gcode` files; transferring them is manual.
- **Real U1 cascade** — only the A1 mini cascade is bundled
  today. Authoring a real U1 cascade is post-MVP profile work.
- **Mode-filter + diff-tab redesign** — the surfaces are
  parked pending a UX redesign; mode is pinned to `advanced`
  and diff to `all` until then.
- **Frontend smoke (`exit_smoke.test.ts`)** — needs a DOM
  testing harness (jsdom + @testing-library) we haven't set
  up. The pure projections + invoke wrappers are covered by
  the per-component vitest cases.

## Cleanup

```
$ rm ~/n3o-test-3plate.3mf
$ rm -rf ~/.local/share/n3o-slic3r/autosave/
```
