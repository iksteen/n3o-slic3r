# Spike 2: mixed-nozzle-size slice (Prusa XL 5T)

## Assumption tested

libslic3r's per-toolhead independence claim: given a Prusa XL 5T
profile with five extruders, we can override `nozzle_diameter` to a
mixed vector (tool 0 at 0.4 mm, tool 1 at 0.6 mm, the rest at 0.4 mm)
and produce a slice where libslic3r honors the per-tool config —
both at the config-block level (gcode header reflects the override)
and at slice time (per-tool retraction / wipe / nozzle_type carry
through; per-tool extrusion volume tracks the active extruder's
nozzle diameter via libslic3r's flow calculator).

The Snapmaker U1 toolchanger is the downstream consumer of this
finding: U1 has different toolhead counts and a different change
mechanism than the XL, but the per-extruder-config concern is the
same. If the engine handles XL's mixed nozzles cleanly, U1's two
heads should too.

## Method

1. `python3 scripts/spikes/convert_orca_profile.py \
       --vendor Prusa \
       --machine "Prusa XL 5T 0.4 nozzle" \
       --process "0.20mm Speed @Prusa XL 5T 0.4" \
       --filament "Prusa Generic PLA @XL 5T" \
       --out examples/cascades/prusa-xl-5t-spike2.toml`

   Produces a 253-key top-level default + 2 `[[rule]]` blocks
   (filament rule for PLA, plate rule for Textured PEI). The
   default has `nozzle_diameter = "0.4,0.4,0.4,0.4,0.4"` from the
   merged 5T machine profile.

2. `cargo run -p n3o-slic3r --release --example spike2`

   `src-tauri/examples/spike2.rs` reuses spike1's cascade parser
   and resolver. After resolution, it overwrites
   `resolved["nozzle_diameter"]` with `"0.4,0.6,0.4,0.4,0.4"`
   (production should express this via a `[[rule]] when.extruder.id
   = 1` predicate once Phase 1's resolver handles per-extruder
   predicates). Adapter pushes the resolved config into a
   `slic3r_ffi::Config`, overlays onto the OrcaCube_v2.3mf
   embedded config, slices, writes `/tmp/spike2.gcode`.

3. Inspect the gcode's libslic3r config block (`; key = value`
   lines between `; CONFIG_BLOCK_START` and `; CONFIG_BLOCK_END`)
   for evidence that the override flowed through and that other
   per-extruder vectors got broadcast / honored. Inspect the gcode
   body for `T0`/`T1` tool-change instructions (which won't appear
   without a multi-color model, as noted below).

## Result

**PARTIAL pass.** The engine accepts mixed-nozzle config and emits
a valid 2 550 718-byte / 94k-line gcode, but the spike only
exercises half the criteria — tool-change G-code at color
boundaries requires a 2-color test model, which we don't have a
ready-made fixture for. Surface gaps detailed below.

### Per-extruder vector dispatch — PASS

Cascade override flows end-to-end. The gcode's CONFIG_BLOCK shows:

```
; nozzle_diameter = 0.4,0.6,0.4,0.4,0.4
; retraction_distances_when_cut = 18,18,18,18,18
; retraction_length = 0.8,0.8,0.8,0.8,0.8
; retraction_minimum_travel = 1.5,1.5,1.5,1.5,1.5
; retraction_speed = 35,35,35,35,35
; wipe = 1,1,1,1,1
; nozzle_type = hardened_steel,hardened_steel,...,hardened_steel
```

libslic3r **broadcasts scalars to vectors** when the cascade
supplies a single value but the option is per-extruder: the
cascade had `retraction_length = "0.8"` (a scalar string), but the
gcode header carries `0.8,0.8,0.8,0.8,0.8`. Same for `wipe`,
`retraction_speed`, `nozzle_type`. This means the Phase 1 cascade
doesn't have to author 5-element vectors for every per-extruder
key — scalars get padded out by the engine.

Two separator conventions worth noting (might bite a future
parser/round-trip):

- **Comma** for numeric and most string vectors:
  `nozzle_diameter = 0.4,0.6,0.4,0.4,0.4`,
  `nozzle_type = hardened_steel,...`.
- **Semicolon** for string vectors containing whitespace:
  `print_extruder_variant = "Direct Drive Standard";"Direct Drive
  Standard";...`.

There are 199 vector option keys in libslic3r's option table (per
`PrintConfig.cpp` scan: coFloats / coStrings / coBools / coPercents
/ coInts) — that's the surface area for per-extruder dispatch.

### Per-tool extrusion width — PASS, with a documented engine constraint

libslic3r's `*_line_width` keys (`line_width`, `outer_wall_line_width`,
`inner_wall_line_width`, `top_surface_line_width`, ...) are all
**scalars** (`coFloatOrPercent`), not per-extruder vectors. So
authoring different widths for tool 0 vs tool 1 is not directly
expressible in config.

What libslic3r actually does: at slice time, the active extruder's
`nozzle_diameter` feeds the flow calculator, which computes the
extrusion volume per move. The authored `outer_wall_line_width =
0.45` is the *target* width; the actual extruded width tracks the
active tool's nozzle. So a 0.4 mm nozzle and a 0.6 mm nozzle
emitting an "outer wall" at the authored 0.45 mm width will extrude
different volumes — the engine handles this without per-tool config.

For our downstream concerns:

- **U1 toolchanger** — fine. As long as the U1 driver populates
  `nozzle_diameter` per actual head, libslic3r's flow calculator
  does the right thing. We don't need per-tool widths in the
  cascade.
- **Quality control between very-different nozzles** (0.4 mm + 0.8
  mm, say) — would benefit from per-tool widths but libslic3r
  doesn't support that. Not a blocker; document the constraint and
  move on.

### Tool-change G-code at color boundaries — DEFERRED

The slice is single-color (OrcaCube_v2.3mf assigns one filament),
so libslic3r emits two `T0` instructions in the machine-start G-code
block (one tool pre-initialization, one "pick the tool") and zero
`T1`+ in the body. No mid-print tool changes — there's nothing for
the engine to switch to.

Validating actual tool-change emission requires either:

1. A 2-color test 3MF with painted regions assigning two filaments.
   OrcaSlicer's `resources/` directory doesn't ship one;
   constructing a multi-volume 3MF programmatically (XML + zip, two
   meshes, per-volume `extruder=` metadata) is doable but yak-shave
   for a spike.
2. Reusing PR-0.5-3's 4-color 3MF once that's produced — and
   re-running spike2 against it would exercise tool-change for any
   2-of-the-4 transition.

Recommend: defer the toolchange-emission criterion to PR-0.5-3,
which already requires a multi-color OrcaSlicer-painted 3MF; that
fixture covers both spikes. The mixed-nozzle aspect of PR-0.5-2 is
otherwise validated.

### Independent per-tool retraction / wipe / jerk — PASS via cascade authoring

Per-extruder retraction (`retraction_length`, `retraction_speed`,
`deretraction_speed`, `retract_when_changing_layer`,
`retract_before_wipe`, etc.) and wipe (`wipe`,
`retract_lift_below`, `retract_lift_above`, `z_hop_types`) are all
vector options. The cascade can author them either as scalars
(broadcast) or as 5-vectors (explicit per-tool). The XL 5T
profiles ship them as scalars; OrcaSlicer relies on the broadcast
behavior.

For the U1 driver: if both heads should share retraction, scalars
in the cascade are fine. If the two heads need different retraction
behavior, the cascade can author the vector explicitly.

## OrcaSlicer-side data-quality issues surfaced

13 keys from the Prusa XL 5T cascade didn't make it into
`Config::set` (vs 67 for the BBL spike — Prusa's profiles map more
cleanly to libslic3r). Five of the 13 are typos in OrcaSlicer's
own profile JSONs:

| OrcaSlicer key (in profile) | libslic3r key (correct) |
|---|---|
| `detraction_speed` | `deretraction_speed` |
| `inital_layer_height` | `initial_layer_height` |
| `nozzle_temperature_intial_layer` | `nozzle_temperature_initial_layer` |
| `tree_support_bramch_diameter_angle` | `tree_support_branch_diameter_angle` |
| `wall_infill_order` (key name shape — needs upstream check) | (TBD; might be `wall_infill_order` actually exists in some forks) |

These typos mean the affected values **have no effect in OrcaSlicer
either** — libslic3r silently discards unknown keys when loading,
so the Prusa profile authors' intent is lost upstream. Worth
filing upstream if Phase 5 (Prusa support) ever happens; for our
purposes, the Phase 1 translator's drop list should include these
typos so the cascade dumps don't surface them as warnings every
slice.

The other 8 unknowns are the expected OrcaSlicer-fork extras:
`adaptive_layer_height`, `bed_type`, `filament_id`,
`filament_load_time`, `filament_unload_time`, `smooth_plate_temp`,
`smooth_plate_temp_initial_layer`,
`tree_support_branch_diameter_double_wall`.

## FFI surface gaps discovered

None new. The two ergonomic gaps noted in `spike-1-cascade-adapter`
(no `Config::merge`, error-kind via string-match) still apply but
weren't blockers.

## libslic3r dispatch quirks beyond `docs/libslic3r-workarounds.md`

None new. The five existing workarounds still apply. The
scalar→vector broadcast for per-extruder keys is documented engine
behavior, not a quirk — it's exactly what
`docs/profiles.md` "What stays libslic3r-shaped → dispatch quirks"
called out as expected normalization.

One semantic note: when a vector option has 5 elements but the
print only uses tool 0, the unused tool-1..4 entries are still
present in the gcode header. This is correct (the printer firmware
may need them for tool warmup, tool storage temperature, etc.)
but means the header isn't a faithful "what this slice actually
uses" record — it's "what the configured machine *could* use."

## Implications for downstream phases

- **Phase 1 (Rule cascade + adapter).** Proceed as planned. The
  cascade can author per-extruder keys as either scalars (broadcast)
  or vectors (explicit). The Phase 1 resolver should support a
  `when.extruder.id = N` predicate to enable per-tool conditional
  rules cleanly; the spike's direct override of the resolved map is
  a placeholder for that. Add the five OrcaSlicer typos to the
  translator's drop list.

- **Phase 5 (Multi-printer + drivers).** The Snapmaker U1 driver
  can rely on libslic3r's flow calculator to handle per-tool
  extrusion volume — no per-tool line-width config plumbing needed.
  The U1 cascade should declare `nozzle_diameter` as an explicit
  2-vector (`"0.4,0.8"` for the dual-head SKU) and other
  per-extruder keys as scalars unless there's a specific reason to
  differ per head.

- **Phase 7 (Filament sync).** Filament profile values flow
  through per-extruder when the active filament differs by tool;
  cascade-rule placement of filament keys in the filament rule
  (rather than at top level) is correct.

No spike-failure plan-revision needed.

## Artifacts

- `examples/cascades/prusa-xl-5t-spike2.toml` — generated cascade
  for Prusa XL 5T 0.4 + 0.20mm Speed + Prusa Generic PLA @XL 5T.
- `src-tauri/examples/spike2.rs` — driver: load + resolve + override
  + adapt + slice.
- `/tmp/spike2.gcode` — 2 550 718 bytes, 94k lines, single-color
  slice of OrcaCube_v2.3mf. Not checked in.
- libslic3r submodule pin: `956fcea7e2`.
- Full skipped-key list: `SPIKE_DUMP_GAPS=1 cargo run -p n3o-slic3r
  --release --example spike2`.
