# Phase 8 — platecycler hardware smoke

> Status: **method written; awaiting a real-hardware run.** This is the
> exit-criteria proof for the plugin architecture (the reduced FR-PL-5
> proof point — see `docs/tickets/phase-8.md`): a post-slice plugin
> appends the Chitu PlateCycler eject macro so the finished plate is
> auto-ejected on print completion.

## Assumption tested

The `platecycler` example plugin (`plugins/platecycler/`), run
at post-slice, appends the `DEFAULT_SWAP_GCODE` eject/swap macro to the
tail of a real A1 mini slice such that, when the print finishes on the
project lead's A1 mini + Chitu PlateCycler, the macro runs and the plate
is swept off automatically.

## Pre-flight (do this first)

⚠️ **Verify the macro before sending to hardware.** Diff
`plugins/platecycler/main.lua`'s `SWAP_GCODE` against your
current `platecycler.py` `DEFAULT_SWAP_GCODE`
(github.com/iksteen/platecycler). The macro drives the toolhead through
a fixed ejection path; a wrong coordinate can crash the toolhead into
the bed. The plugin is opt-in (not auto-loaded) precisely so this
verification happens deliberately.

## Method

1. platecycler **ships bundled** (`plugins/platecycler/`, installed via
   `tauri.conf.json` resources; loaded in dev through
   `N3O_PLUGIN_ROOT=./plugins`), so no install step is needed. Like all
   plugins it's **off by default** — enable it from the Plugins panel
   (the brand menu's "Global plugins…", or per-plate via the settings
   panel's Plugins tab).

2. Load a small model, bind the plate to the **Bambu Lab A1 mini**, and
   slice. The plugin's printer self-guard only fires for that model.

3. **Inspect before sending:** export the sliced `.gcode` / preview and
   confirm `; n3o:platecycler` + the `G0 …` / `G4 …` eject sequence
   appear **just before `; EXECUTABLE_BLOCK_END`** (inside the runnable
   block — Bambu firmware ignores anything past END), after the slice's
   own end-G-code.

4. Send to the A1 mini and let the print complete. Watch the end of the
   job: the eject sequence should run and the PlateCycler should sweep
   the part off the plate.

5. Slice + send a second job to confirm a clean plate was staged and
   the cycle repeats.

## Expected result

- The exported G-code carries the sentinel + the full macro once, just
  before `; EXECUTABLE_BLOCK_END` (idempotent — re-running the hook
  never double-inserts; covered by the
  `platecycler_inserts_eject_macro_inside_executable_block_idempotently`
  integration test).
- On hardware, the finished plate is auto-ejected at print end with no
  manual intervention.

## Result

**✅ Confirmed (2026-05-31).** The project lead ran a sliced print with
the platecycler plugin active on the A1 mini + Chitu PlateCycler. The
eject/swap macro ran at print end and the finished plate was swept off
automatically with no manual intervention — the Phase 8 exit-criteria
proof point that the plugin architecture works end-to-end on real
hardware.
