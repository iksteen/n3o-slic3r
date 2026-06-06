# PR-S exit-criteria smoke

Locks the multi-instance + multi-filament work from PR-S-1 through
PR-S-10 into a single repeatable test. If the cascade composer
regresses filament fan-out, per-OptType separator dispatch, or
`flush_volumes_matrix` sizing, this fails loudly before a real slice
ever runs.

## Automated half — runs in CI

```
$ cargo test -p n3o-slic3r --test phase_s_smoke
```

Expected:

| Suite          | Tests | Notes                                                      |
| -------------- | ----- | ---------------------------------------------------------- |
| phase_s_smoke  |  2 + 1 ignored | bambi + snappy slice the 4-color benchy; leg 3 deferred |

Each leg slices `examples/spike3/fourcolor.3mf` (the 4-color benchy
AMS test, CC BY-NC 4.0 — see `examples/spike3/NOTICE.md`) on a
different printer instance and asserts multi-filament tracking +
the appropriate gcode swap-marker shape.

## What `phase_s_smoke.rs` exercises

Two active tests plus one `#[ignore]`d placeholder, covering the
three legs from `docs/dev/settings-model.md` §11.4.

### Leg 1 — `bambi_multi_color_slices_with_filament_tracking`

Slices the fixture on bambi (Bambu Lab A1 mini, 1 extruder × 5 AMS
slots) via `run_slice_job_blocking`. Asserts:

1. **Slice completes** — `PlateFinished` arrives, no `JobFailed`.
2. **Multi-filament tracking** — `summary.filament_used_grams`
   contains ≥2 non-zero entries. Single-color fallback would surface
   as one non-zero entry; ≥2 confirms the composer's filament fan-out
   reached libslic3r and per-slot accounting is live.
3. **AMS swap macros** — gcode contains ≥1 `M620 SnA` line. This is
   Bambu's "swap material to AMS slot n" marker — one per real
   filament change in the print body. Zero would mean the slicer
   collapsed the multi-color regions back to one filament.

Exercises: filament fragment fan-out (PR-S-5), `filament_colour`
synthesis from PrinterInstance slot colors (this PR), and the
`flush_volumes_matrix` resize from the libslic3r default 4×4 to
the 5×5 required by bambi's 5-slot topology.

### Leg 2 — `snappy_multi_color_slices_with_toolhead_changes`

Slices the same fixture on snappy (Snapmaker U1, 4 toolheads ×
1 slot each) via the same path with `printer_instance_id =
"snappy"`. Together with leg 1 this confirms the per-job instance
binding routes correctly — distinct `PrinterInstance`s in the same
process slice the same model with different cascade compositions.
Asserts:

1. **Slice completes** — `PlateFinished` arrives, no `JobFailed`.
2. **Multi-filament tracking** — same shape as leg 1; ≥2 non-zero
   entries.
3. **Toolhead changes** — gcode contains ≥4 bare `T<n>` lines.
   start_gcode emits 3× `T0` (dock-initial); the print body adds
   real swaps as the slicer cycles between toolheads. Anything
   below 4 means either start_gcode shrank or body-side swaps
   didn't survive the slice.

Exercises the same composer plumbing as leg 1, but against the
toolchanger topology (4 extruders × 1 slot vs 1 extruder × 5
slots).

### Leg 3 — `copy_vs_vendor_binding_is_independent` (`#[ignore]`d)

Placeholder. The in-app filament/process copy mechanic is a
tracked MVP exclusion (`settings-model.md` §9 — "In-app
filament/process copy UX"). Once that surface lands, replace this
with: copy a vendor filament, mutate the copy, slice with the
copy, assert the vendor source is unchanged and the slice picked
up the override.

Until then the test is `#[ignore]`d with a comment pointing at the
gap, so the expectation stays visible without breaking CI.

## Tree-support override

Both legs author a project-tier override:

```toml
enable_support = "1"
support_type = "tree_auto"
```

The 3DBenchy has floating regions (bow overhang) and libslic3r
refuses to slice it without supports — without the override the
job ends with `It seems object … has floating regions. Please
re-orient the object or enable support generation.` We bypass that
by enabling tree-auto for the smoke. The override doesn't change
what we're testing (cascade composition + multi-filament path), it
just gives the slicer a way to handle the model's geometry.

## Human-driven half

Beyond the automated checks above, the following needs eyeballs
before a phase-S tag goes out:

1. **Launch the app** (`npm run tauri dev`), create a new
   project, add a bambi printer instance from the empty state.
2. **Slice a single-color cube** via the main viewport. Confirm:
   the Slice button activates → progress events stream in the
   panel → `PlateFinished` shows a preview thumbnail.
3. **Add the snappy instance** from the printer picker, switch
   the active plate to it, slice the same cube. Confirms the
   instance-switch path works in the UI, not just the test
   harness.
4. **Load `4colorbenchy.3mf` or another multi-color model**, bind
   each AMS slot to a distinct generic-PLA filament with
   different colors, slice on bambi. Confirm: the preview shows
   the model in the chosen colors and the gcode header reports
   ≥2 non-zero `filament_used_grams` entries.

If any human-driven step fails but the automated tests pass, that
means the UI-side wiring around the slice path regressed without
the orchestrator noticing — likely a Tauri command shape or event
listener issue.
