# PR-1-12 — Tool-change minimization investigation (from PR-0.5-3)

Status: ❌ open (carried from PR-0.5-3).

**Scope.** Investigate and fix the disparity surfaced by PR-0.5-3:
on the 4-color Benchy AMS input, our FFI-driven slice emits 76
mid-print tool changes (2h 55m print, ~25 g filament) while
OrcaSlicer-app and Bambu Studio both emit 7 (1h 3m print, ~14 g)
using the same engine code. The cause is in *how we invoke
libslic3r*, not the engine itself — see
`docs/spikes/spike-3-bambu-ams.md` for the full chain.

**Why a Phase 1 ticket** (not Phase 5): the adapter (PR-1-6) is
the right place to apply whatever pre-`apply` setup is missing.
Touching adapter code with the investigation still cold risks
re-introducing the gap. Land the fix while the spike context is
fresh.

**Acceptance criteria.**

Investigation deliverables (must complete; pick at least one):

- **Diff `WipingExtrusions` state.** Instrument OrcaSlicer CLI
  and our FFI to dump
  `wiping_extrusions.get_support_extruder_overrides(object)` for
  the 4-color Benchy input right before
  `GCode::process_layer`. If OrcaSlicer populates this and we
  don't, that's the gap — likely a missing call somewhere in the
  pre-apply / pre-process flow.
- **Trace `layer_tools.extruders.front()` in
  `GCode.cpp:4794-4820`.** If OrcaSlicer's per-layer extruder
  list starts with the band's body extruder and ours starts with
  T0 (carried from prior band), the bug is in tool-ordering
  setup. Look for the OrcaSlicer-side code that sets up
  `layer_tools` differently.
- **Diff per-`ModelVolume` config.** PR-0.5-3 confirmed 5
  PrintRegions (4 with wall_filament=1..4, 1 with =0). If
  OrcaSlicer's flow has 4 regions (no zero), there's a
  pre-apply step that pushes per-volume `extruder` metadata
  into each volume's config to suppress the zero region.

Fix deliverables:

- Whatever the investigation surfaces, the fix lives in
  `core/cascade_adapter/` (or the FFI shim if it's truly an
  engine-invocation quirk, per the `docs/libslic3r-workarounds.md`
  pattern). Document it as workaround #6 (or #5 update) in that
  file.

- Re-run spike3 (`cargo run --release --example spike3`) and
  confirm the tool-change count drops to ≤ 10. The exact number
  may vary (libslic3r version differences vs OrcaSlicer 2.4.0-dev)
  but the order of magnitude matches Bambu Studio / OrcaSlicer.

- Re-run spike1 and spike2 to confirm no regression on the
  single-color / mixed-nozzle cases.

- Update `docs/spikes/spike-3-bambu-ams.md` "Investigation so far"
  with the resolved cause and the diff link.

- Phase 5 prerequisite section in the spike3 finding flips from
  "must be solved before Phase 5 hardware validation" to "solved
  in PR-1-12 (commit X)".

**Effort.** ~2-3 days investigation, 1-2 days fix + integration.
Could blow up if the cause is structural (e.g., a missing
pre-apply phase that OrcaSlicer hides in its preset bundle); in
that case, surface options to the user before going deeper rather
than re-implementing the entire flow.

**Dependencies.** Independent of other Phase 1 tickets in the
sense that the investigation can start immediately. The *fix*
should land alongside PR-1-6 (adapter) so the regression test
makes sense.

**Out of scope.** Bambu Studio's preset-bundle resolution
mechanism (we deliberately don't replicate it; the goal is the
*output*, not the implementation). Other multi-color quirks
beyond tool-change minimization (e.g. wipe tower placement) —
those land as Phase 5 prerequisites if they surface.
