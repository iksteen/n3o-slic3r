# PR-5-12 — Phase 5 exit-criteria smoke

Status: ❌ open.

**Scope.** End-to-end smoke that exercises Phase 5's exit
criterion as a single repeatable test. Mirrors `phase-3-smoke.md`
+ `phase-4-smoke.md` — half automated (Rust + frontend
tests), half human-driven (the App.tsx multi-plate
workflow).

Phase 5's exit criterion is the **3-plate save/reload**
proof:

> Create a 3-plate project, assign Plate 1 to A1 mini and
> Plates 2-3 to U1, slice all three, save and reload with
> all settings preserved including per-plate cycle counts
> and material bindings.

**Acceptance criteria.**

- `docs/dev/phase-5-smoke.md` documents:
  1. **Automated half** — `cargo test --workspace` +
     `npm test` cover the structural gates.
  2. **Manual half — 3-plate save/reload walkthrough:**
     - Open `npm run tauri dev`.
     - From the bundled empty project, add 2 more plates
       via the `+` tab affordance.
     - Bind Plate 1 → A1 mini (default), Plates 2-3 → U1.
       (Stub U1 printer profile if not yet authored.)
     - Add a cube to Plate 1; load fourcolor.3mf onto
       Plate 2; load 20mmbox-LF onto Plate 3.
     - Override `layer_height = 0.12` at the project
       tier on Plate 2; set the cube on Plate 1 to
       `enable_support = true` (object-tier).
     - Set Plate 2's cycle count to 3; Plate 3's to 2.
     - Slice all three plates sequentially.
     - Save the project to `~/n3o-test-3plate.3mf`.
     - Close the app; reopen.
     - Load the saved project.
     - Verify: 3 plates, correct printer bindings,
       correct project + object overrides, correct cycle
       counts, all settings panel rows resolve to the
       same values as before the save.

- `src-tauri/tests/phase5_smoke.rs` (automated half):
  1. Build the 3-plate project programmatically (no UI).
  2. Set the same overrides + cycle counts + material
     bindings the manual walkthrough sets.
  3. Save to a temp file via `project_save`.
  4. Drop the in-memory project; load the saved file
     via `project_load`.
  5. Assert: plate count, printer bindings, overrides,
     cycle counts, material bindings all round-trip
     byte-equivalent (assert per-field equality).
  6. Slice plate 1 + plate 2 + plate 3 (use existing
     fixtures); assert each plate produces a `.gcode`
     file and the PR-3-6 parser yields zero `ParseError`.

- `src/plates/__test__/exit_smoke.test.ts` (frontend
  half):
  - Mount the App with a stubbed 3-plate project;
    assert the plate tabs render 3 entries with the
    correct names + printer labels.
  - Switch active plate; assert the settings panel
    re-resolves against the new plate's printer.
  - Click "+ Plate"; assert a new plate appears at the
    end of the tab strip.

- CI: the automated half runs in the existing
  `cargo test --workspace` + `npm test` steps. No new
  CI jobs.

**Effort.** ~1.5 days. The smoke leans on PR-5-1 through
PR-5-10 being shipped; this ticket is mostly composition +
the doc.

**Dependencies.** Every other Phase 5 ticket. Last to
land.

**Out of scope.** Real U1 hardware validation — Phase
7b's hardware test (we have no U1 in the dev rig
today). Multi-user collaboration on a shared project —
post-MVP. Project conversion from foreign slicer
formats beyond geometry (Bambu Studio project →
n3o-slic3r metadata translation) — Phase 9.

**The smoke is the project's gate for the multi-printer
workflow.** If a future change breaks the save/reload
round-trip of any of (plate count, printer bindings,
project overrides, object overrides, material bindings,
cycle counts), the smoke fails — and that preserves
Phase 5's primary differentiator across all future
refactors.
