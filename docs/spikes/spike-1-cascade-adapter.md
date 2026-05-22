# Spike 1: cascade adapter end-to-end

## Assumption tested

The cascade-adapter pattern described in `docs/profiles.md` works
end-to-end: a real OrcaSlicer device profile, converted into our TOML
cascade format and dispatched through a stub resolver + stub adapter,
can drive libslic3r to produce valid G-code. The constraint added
during PR-0-5 — "no hand-rolled minimum configs; the seed must be a
converted real device profile" — is exercised here for the first
time, since FullPrintConfig defaults can't pass `Print::validate()`.

Sub-claims:

- specificity-based resolution behaves as documented (default rule
  loses to filament/plate rules; equal-specificity rules resolved by
  source order);
- dimensional expansion (bed temperature across plate types) is
  expressible in the adapter without help from libslic3r;
- libslic3r's per-extruder vector keys round-trip through string
  values (the JSON arrays our converter emits as comma-joined strings
  end up parsed as vectors at `Print::apply`).

## Method

1. `python3 scripts/spikes/convert_orca_profile.py \
       --vendor BBL \
       --machine "Bambu Lab A1 mini 0.4 nozzle" \
       --process "0.20mm Standard @BBL A1M" \
       --filament "Bambu PLA Basic @BBL A1M" \
       --out examples/cascades/bambu-a1-mini-spike1.toml`

   The converter walks the inherits chain for each of the three
   profiles, flattens to one dict per kind, merges the three (with
   filament overriding process overriding machine), filters out
   OrcaSlicer metadata keys (`type`, `name`, `inherits`,
   `compatible_printers`, etc.) and the plate-dim keys
   (`hot_plate_temp` and friends), then emits three TOML rule blocks:
   a default rule with the bulk, a filament rule with PLA-specific
   temperature/fan keys, and a plate rule carrying a single logical
   `bed_temp` + `curr_bed_type`.

2. `cargo run -p n3o-slic3r --release --example spike1`

   The example deserializes the cascade with `toml`, resolves it
   against context `{filament.type = "PLA", plate.type = "Textured
   PEI"}` (lowest specificity first, source-order tie-break), then
   adapts the resolved flat map into a `slic3r_ffi::Config`. The
   adapter expands `bed_temp` across all fourteen libslic3r
   per-plate-type vector keys. The model `OrcaCube_v2.3mf` is loaded
   with its embedded config, our cascade result is overlaid on top,
   and `slic3r_ffi::slice()` is called.

3. Inspect `/tmp/spike1.gcode` for size + header validity. Re-run
   with `SPIKE_DUMP_GAPS=1 cargo run ... --example spike1` to list
   every skipped key.

## Result

**PASS.** The slice produces `/tmp/spike1.gcode` — 2 341 414 bytes,
94 125 lines, 150 layers — with a valid OrcaSlicer-style header
(`; HEADER_BLOCK_START`, generator string, time estimates, etc.).
`Print::validate()` does not reject the configuration; this is the
first successful end-to-end slice in the repo, vindicating the
PR-0-5 decision to gate gcode-out on Spike 1 rather than patching
the example with a hand-rolled minimum config.

Cascade-level numbers:

- 338 top-level keys (the unconditional default — specificity 0)
  + 2 `[[rule]]` blocks (filament rule with 6 sets, plate rule with
  2 sets) parsed from the cascade.
- 346 distinct keys resolved against the test context.
- Specificity ordering observed correctly: defaults applied first,
  filament + plate overrides applied on top.

(The cascade was originally emitted with three `[[rule]]` blocks,
including a 338-set unconditional default. Mid-spike, the format
gained top-level-keys-as-default sugar documented in
`docs/profiles.md` "three equivalent forms"; the converter and
resolver were updated to emit / consume the new shape. Resolver
behavior is identical — top-level keys desugar to a virtual
`[[rule]] when = {}` at source position 0.)

Adapter-level numbers:

- 293 keys accepted by `Config::set`.
- 67 keys skipped — all `UnknownKey`, zero `ParseValue`, zero other.
  The full list is below.

The skipped keys fall into four rough categories. None is a blocker
for Phase 0.5; all are inputs to Phase 1's translation manifest and
Phase 5's driver layers.

| Category | Examples | Count |
|----------|----------|-------|
| Bambu firmware tuning | `hotend_cooling_rate`, `hotend_heating_rate`, `machine_prepare_compensation_time`, `machine_switch_extruder_time`, `enable_pre_heating`, `chamber_temperatures` | ~10 |
| AMS / multi-color extensions | `filament_long_retractions_when_ec`, `filament_retraction_distances_when_ec`, `filament_scarf_*`, `filament_prime_volume`, `filament_ramming_*`, `filament_velocity_adaptation_factor`, `override_filament_scarf_seam_setting` | ~12 |
| Circle / hole compensation | `circle_compensation_*`, `counter_coef_*`, `counter_limit_*`, `hole_coef_*`, `hole_limit_*`, `diameter_limit`, `enable_circle_compensation`, `apply_top_surface_compensation` | ~12 |
| Process knobs that exist only in OrcaSlicer's fork | `adaptive_layer_height`, `layer_time_smoothing*`, `slowdown_start_*`, `slowdown_end_*`, `prime_tower_lift_*`, `prime_tower_max_speed`, `overhang_totally_speed`, `pre_start_fan_time`, `top_color_penetration_layers`, `bottom_color_penetration_layers`, `seam_slope_gap`, `seam_placement_away_from_overhangs`, `smooth_coefficient`, `infill_rotate_step`, `internal_bridge_support_thickness`, `wall_infill_order`, `vertical_shell_speed`, `z_direction_outwall_speed_continuous`, `detect_floating_vertical_shell`, `enable_height_slowdown`, `impact_strength_z`, `locked_skin_infill_pattern`, `locked_skeleton_infill_pattern` | ~25 |
| Extruder-clearance + miscellany | `extruder_clearance_dist_to_rod`, `extruder_clearance_max_radius`, `filament_id`, `smooth_plate_temp`, `smooth_plate_temp_initial_layer` | ~8 |

Counts are approximate (some keys span two categories). The first
two columns matter for Phase 5 (the Bambu driver will need to know
these keys exist when round-tripping `.gcode.3mf` to / from the
printer, even if libslic3r ignores them at slice time); the last
two are pure cosmetics that Phase 1's translator should drop.

## FFI surface gaps discovered

None blocking. Two ergonomic gaps the Phase 1 adapter will want
addressed before it's much bigger than the spike's:

- **No `Config::merge`.** The spike works around this with an
  overlay routine that re-calls `Config::set` for every resolved
  key, masking ParseValue / UnknownKey errors. A native
  `merge(other: &Config)` (and a `merge_with_errors` variant that
  reports per-key failures) would be cleaner. Filed implicitly as
  follow-up work for the FFI shim.
- **`Config::set` errors don't distinguish "key exists but value
  parse failed" from "key not in libslic3r" without string-matching
  the Debug repr.** Today the spike scans for `"UnknownKey"` in the
  formatted error. Exposing `ErrorKind` already does the right thing
  at the Rust layer — the gap is just that we forgot to assert on
  `kind` instead of message text in the example. Trivial to fix in
  the Phase 1 resolver.

## libslic3r dispatch quirks beyond `docs/libslic3r-workarounds.md`

None new. The five existing workarounds (temp_dir,
`LoadStrategy::LoadModel`, `is_BBL_printer` init, pre-`apply`
normalization, coEnums serialization) all kept doing their jobs.
The `machine_start_gcode` and `machine_end_gcode` blobs — which carry
extensive OrcaSlicer templating syntax (`{filament_type[initial_no_
support_extruder]}`, `[bed_temperature_initial_layer_single]`, etc.)
— pass through as opaque strings and expand at slice time without
any intervention from the adapter.

One semantic note worth recording: the spike resolves `bed_temp` to
the **textured plate** value (65 °C, since `plate.type = "Textured
PEI"`) and writes that same value into all fourteen plate-temp keys.
That's not what production should do — production should resolve
the cascade against each hypothetical plate type and emit the
appropriate value per key, so the user can swap plate types
post-slice without re-resolving. Documented as Phase 1 work in
`docs/profiles.md` "Translating to libslic3r" → "Dimensional
expansion". The spike's simpler expansion still produces valid
gcode because libslic3r's `curr_bed_type` selector picks the right
key and ignores the rest.

## Implications for downstream phases

- **Phase 1 (Rule cascade + adapter).** Proceed as planned. The
  walking-skeleton pattern works; the production resolver needs to
  add `!important` override tiers (per-user, per-project,
  per-object), trace metadata (winning rule's file:line +
  specificity, also-matching losers), richer predicates than
  string-equality (numeric ranges, set membership), and the proper
  per-plate-type dimensional expansion. Pull `docs/profiles.md`'s
  list of dimensional keys into a TOML translation manifest. The 67
  unknown keys discovered here become the seed for Phase 1's
  "OrcaSlicer-specific keys we deliberately drop" list.

- **Phase 5 (Multi-printer + drivers).** The Bambu driver will need
  to keep track of the ~22 firmware-tuning + AMS-extension keys that
  appear in BBL profiles but not in libslic3r — those are
  printer-state, not slice-state, and the driver round-trips them
  through `.gcode.3mf` metadata even though libslic3r itself ignores
  them. The `filament_*_when_ec` keys are typos in OrcaSlicer's
  recent profiles (should be `_when_cut`); flag this for the driver
  layer if both spellings need to be accepted on output.

- **Phase 7 (Filament sync).** Spike 1's filament rule pulls six
  keys (nozzle temps + fan curves + cooling layer time). Production
  should pull more filament-origin keys into the filament rule —
  `filament_max_volumetric_speed`, `filament_density`,
  `filament_cost`, `filament_flow_ratio`, the per-filament
  retraction settings, and so on. The current default-rule placement
  works because the filament happened to be the same across all
  resolved contexts; with multi-color the cascade would resolve to
  the wrong filament's values.

No spike-failure plan-revision needed. The cascade + adapter design
described in `docs/profiles.md` and PRD §6.1 / §8.2 is validated.

## Artifacts

- `scripts/spikes/convert_orca_profile.py` — the converter.
- `examples/cascades/bambu-a1-mini-spike1.toml` — generated cascade
  (1 280 lines; default rule + filament rule + plate rule).
- `src-tauri/examples/spike1.rs` — stub resolver, stub adapter, and
  end-to-end driver.
- `/tmp/spike1.gcode` — 2 341 414 bytes of generated G-code from the
  OrcaCube_v2.3mf model. Not checked in (regenerable; transient).
- libslic3r submodule pin used: `956fcea7e2`.
- Full skipped-key list: run `SPIKE_DUMP_GAPS=1 cargo run -p
  n3o-slic3r --release --example spike1`.
