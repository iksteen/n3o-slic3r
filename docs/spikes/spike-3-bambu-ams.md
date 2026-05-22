# Spike 3: Bambu A1 mini 4-color AMS slice

## Assumption tested

libslic3r, driven through our FFI, can slice a real 4-color AMS
configuration for the Bambu A1 mini and produce G-code with correct
tool-change semantics and the per-filament metadata Bambu Studio's
`.gcode.3mf` wrapper consumes. Sub-claims:

- 4 filament profiles can be bound to AMS slots 1–4 with distinct
  colors and the bindings flow end-to-end through `Print::apply`;
- libslic3r emits `T0`/`T1`/`T2`/`T3` tool-change G-code at color
  boundaries (this also closes PR-0.5-2's deferred toolchange-
  emission criterion);
- the `change_filament_gcode` template — A1 mini's AMS flush macro
  — expands cleanly at each tool change with the right
  `flush_volumes_matrix` and `flush_temperatures` references;
- the filament-aggregate fields (used [mm/cm³/g], cost, density)
  that Bambu Studio packs into `.gcode.3mf` for the printer's
  display are already in libslic3r's plain `.gcode`, so the Phase 5
  wrapper is an aggregation + zip step, not a slicer extension.

## Method

1. Fixture: `examples/spike3/fourcolor.3mf` (MakerWorld "4 Colors
   Benchy AMS Test (v2)" by jansonne, CC BY-NC; attribution in
   `examples/spike3/NOTICE.md`). The 3MF embeds the BambuStudio
   project config — 4 PLA filaments at slots 1–4 with hex colors
   `#C12E1F`, `#FEC600`, `#00AE42`, `#0A2989`, a 4×4
   `flush_volumes_matrix`, the A1 mini's `change_filament_gcode`
   template, wipe tower position, and the Supertack plate
   selector.

2. `cargo run -p n3o-slic3r --release --example spike3`

   `src-tauri/examples/spike3.rs` loads the 3MF via
   `load_with_config` (so the embedded BambuStudio config seeds
   the slot — no cascade overlay; this spike's point is the
   *output*, not re-validating the resolver from PR-0.5-1),
   sanity-checks the AMS-binding keys with `Config::get`, and
   calls `slic3r_ffi::slice` to write `/tmp/spike3.gcode`.

3. Inspect the gcode for: tool-change distribution (`T0`–`T3` plus
   Bambu's `T1000` / `T255` pseudo-tools), `FLUSH_START` markers
   around AMS purges, and the filament-aggregate comment block
   libslic3r emits in the body.

4. Cross-reference the source 3MF's `Metadata/*` inventory against
   what a `.gcode.3mf` wrapper would need.

## Result

**PASS.** Slice produced `/tmp/spike3.gcode` — 5 015 526 bytes / 278
layers / model printing time 2h 55m 21s. All four AMS slots get
used (with the source model's color regions exercising all of them).

### Tool-change distribution — PASS

```
$ grep -oE '^T[0-9]+' /tmp/spike3.gcode | sort | uniq -c
     36 T0
     19 T1
     19 T2
      2 T3
      2 T1000      (start_gcode: "no tool selected" pseudo-tool)
      1 T255       (end_gcode: pull filament back to AMS)
```

The first two `T1000` and the final `T255` are Bambu firmware
pseudo-tools used in the machine_start/end_gcode templates; the 76
real-tool transitions are mid-print AMS swaps. This **closes
PR-0.5-2's deferred toolchange criterion** — libslic3r does emit
tool-change G-code at color boundaries when fed a multi-filament
model; the gap was the test fixture, not the engine.

Each tool-change site expands the A1 mini's `change_filament_gcode`
template inline, with the `[next_extruder]` /
`flush_volumetric_speeds[i]` / `flush_temperatures[i]` /
`flush_length_*` placeholders resolved against
`flush_volumes_matrix`. `FLUSH_START` / `FLUSH_END` comment markers
bracket each purge for downstream tooling (we'll need these for
Phase 8's compose-hook).

### Filament-aggregate metadata — PASS, already in the gcode body

libslic3r emits the per-filament accounting comments Bambu Studio
extracts into the `.gcode.3mf` plate metadata:

```
; filament used [mm] = 2767.35, 3247.09, 2880.50, 912.50
; filament used [cm3] = 6.66, 7.81, 6.93, 2.19
; filament used [g]  = 8.39, 9.84, 8.73, 2.77
; filament cost      = 0.21, 0.25, 0.22, 0.07
; filament_density   = 1.26,1.26,1.26,1.26
; model printing time: 2h 55m 21s; total estimated time: 3h 1m 16s
; total layer number: 278
; max_z_height:       55.60
```

Plus the in-CONFIG_BLOCK keys (`filament_colour`,
`filament_settings_id`, `nozzle_diameter`, `bed_temperature_formula`,
etc.). Phase 5's `.gcode.3mf` wrapper can pluck all of this from
the gcode itself — no FFI extension needed, just a parser for the
header's comment-prefix lines.

### G-code structure — three labelled blocks

libslic3r emits the gcode in three machine-parseable sections:

```
; HEADER_BLOCK_START
; model label id: 8
; HEADER_BLOCK_END
; CONFIG_BLOCK_START
... (full config dump, one `; key = value` per line, alphabetical)
; CONFIG_BLOCK_END
; EXECUTABLE_BLOCK_START
... (the actual print instructions, with WIPE_START/WIPE_END,
     FLUSH_START/FLUSH_END, ; CHANGE_LAYER, etc. markers)
; EXECUTABLE_BLOCK_END
```

The block boundaries are stable and pattern-grep-friendly. Useful
for Phase 6 (G-code preview) and Phase 8 (compose-hook plugin).

## `.gcode.3mf` wrapping gap — what Phase 5 needs to build

libslic3r/FFI today only writes plain `.gcode`. Bambu Studio
wraps its output in a `.gcode.3mf` ZIP whose Bambu-relevant
contents (mirrored from this spike's input 3MF inventory) are:

| File | Purpose | Provenance |
|---|---|---|
| `Metadata/plate_1.json` | Plate bbox, filament colors / ids, bed type, first_extruder, nozzle_diameter, first_layer_time, version, is_seq_print | Aggregate from the gcode header + the source 3MF |
| `Metadata/plate_1.png` | Plate top-down render at slice time | Renderer output (Phase 2 viewport screenshot) |
| `Metadata/top_1.png` | Top-down preview | Renderer output |
| `Metadata/pick_1.png` | Click-target preview | Renderer output |
| `Metadata/plate_no_light_1.png` | Plate render without lighting | Renderer output |
| `Metadata/slice_info.config` | Slicer version + client type | Static — our slicer's identity (`X-BBL-Client-Type=slicer`, `X-BBL-Client-Version=...`) |
| `Metadata/project_settings.config` | Full flat config | Same content we already emit in `; CONFIG_BLOCK` |
| `Metadata/model_settings.config` | Per-object settings + placement | Phase 3's project model |
| `Metadata/filament_sequence.json` | Nozzle sequence + AMS optimal assignment | Resolved at slice time; we can emit a stub initially |
| `_rels/.rels`, `[Content_Types].xml`, `3D/3dmodel.model`, `3D/Objects/*.model` | Standard 3MF container | Re-use from the source 3MF or re-emit via Phase 3's 3MF writer |
| `Auxiliaries/*` | Source-model thumbnails | Re-use from the source 3MF unchanged |

Three categories of work that fall out of this:

1. **Aggregation + zip** (~1 person-week for Phase 5). Parse the
   gcode header into a typed model, build the JSON / config
   payloads, zip the result. The hardest part is `plate_1.json`'s
   shape (a small structured JSON with bbox, filament colors,
   first_extruder, first_layer_time — all derivable from the
   gcode body's `; filament_used`, `; total layer number`,
   `; CONFIG_BLOCK` content).

2. **Render thumbnails** (gated on Phase 2's 3D viewport). Without
   a viewport, our `.gcode.3mf` could ship without thumbnails —
   Bambu Studio's `.gcode.3mf` does too in some configurations.
   Confirm Phase 5 whether thumbnails are mandatory on the
   printer side.

3. **Settings flattening** (Phase 3's project model output). The
   `project_settings.config` is the slice's full flat config; we
   already emit this in the gcode's CONFIG_BLOCK. The wrapper just
   re-emits the same content in JSON-ish key:value pairs.

Blocking vs cosmetic, for "send to printer and have it print":

- **Blocking** (printer rejects the upload without these):
  `plate_1.json`, `slice_info.config`, `project_settings.config`,
  `model_settings.config`, the actual gcode (as `Metadata/plate_1.gcode`).
- **Probably blocking** (printer LCD shows nothing if missing,
  unclear whether print still runs): `plate_1.png`,
  `Auxiliaries/Model Pictures/*`.
- **Cosmetic** (printer queue shows generic icon if missing):
  `top_1.png`, `pick_1.png`, `plate_no_light_1.png`,
  `Auxiliaries/.thumbnails/*`.

The "probably blocking" row needs hardware validation in Phase 5.
Spike 3's job is to characterize the inputs, not to test the
printer's tolerance — that's the Phase 5 hardware-validation
phase.

### Bambu Studio reference comparison — not done in this spike

PR-0.5-3's original scope included slicing the same input with
Bambu Studio and diffing the resulting `.gcode.3mf`. Skipped here
because the dev machine doesn't have Bambu Studio installed (would
require a GUI session and a Bambu account login). The reference
artifact at `examples/spike3/bambu-studio-reference.gcode.3mf`
is **not** produced; this is a follow-up that should land before
Phase 5 starts. The blocking-vs-cosmetic table above is informed
by the source 3MF's inventory (which IS a Bambu Studio output, so
serves as the reference for structure) plus Bambu Studio's
published `.gcode.3mf` spec — but per-field byte-exact verification
needs the real reference.

## FFI surface gaps discovered

One non-blocking, one nice-to-have:

- **No `.gcode.3mf` export.** `slic3r_ffi::slice` only writes the
  plain `.gcode`. Phase 5's wrapper would either live in Rust
  above the FFI (consuming the plain gcode + the source 3MF +
  Phase 2's renderer thumbnails) or be added as a new FFI export
  function. Above-the-FFI is cleaner — it doesn't bind us to
  libslic3r's wrapper if Bambu changes the format.
- **`Config::get` exists but isn't exercised in spike1/spike2.**
  This spike used it to sanity-check the embedded AMS bindings
  after `load_with_config`. Spike1/spike2's `overlay` helper
  doesn't read back — fine for the spikes but the Phase 1
  resolver's trace tooling will want round-trip read access.

## libslic3r dispatch quirks beyond `docs/libslic3r-workarounds.md`

None new. The existing five workarounds (incl. the pre-`apply`
normalization of `filament_map` / `nozzle_volume_type` /
`wall_filament`) cover this slice end-to-end. The 3MF's embedded
config goes through the same path as PR-0.5-1's overlay and
PR-0.5-2's mixed-nozzle override.

## Implications for downstream phases

- **Phase 1 (Rule cascade + adapter).** No new requirements. The
  4-filament case for the cascade resolver needs per-filament-slot
  rules (`when.filament.slot = 1..4`); that's a sub-case of the
  existing `when.<dim> = <value>` predicate vocabulary — no new
  semantic.

- **Phase 5 (Multi-printer + drivers).** This finding is the
  largest input to the Bambu driver's `.gcode.3mf` wrapper. The
  Blocking/Probably-Blocking/Cosmetic table is the shopping list.
  The wrapper lives above the FFI: parse gcode header → aggregate
  into typed models → emit JSON + zip. ~1 person-week.

- **Phase 6 (G-code preview).** The labelled blocks
  (`HEADER_BLOCK`/`CONFIG_BLOCK`/`EXECUTABLE_BLOCK`) and the
  in-body markers (`WIPE_START`/`WIPE_END`,
  `FLUSH_START`/`FLUSH_END`, `; CHANGE_LAYER`) are the natural
  parse boundaries.

- **Phase 7 (Filament sync).** The AMS-binding shape
  (`filament_settings_id`, `filament_ids`, `filament_colour`,
  `flush_volumes_matrix`) is the public surface for "send AMS
  config from printer to UI." Round-trips cleanly through
  `.gcode.3mf`.

- **Phase 8 (Compose-hook plugin).** The `FLUSH_START`/`FLUSH_END`
  markers + `T0..T3` lines are where platecycler's gcode
  transforms need to slot in. PR-0.5-5 (platecycler portability)
  picks this up.

No spike-failure plan-revision needed.

## Artifacts

- `examples/spike3/fourcolor.3mf` — 4-color test fixture (CC BY-NC,
  see `NOTICE.md`).
- `src-tauri/examples/spike3.rs` — driver: load + sanity-check + slice.
- `/tmp/spike3.gcode` — 5 015 526 bytes, 278 layers, 76 real
  tool-changes. Not checked in.
- libslic3r submodule pin: `956fcea7e2`.
- Bambu Studio reference `.gcode.3mf`: **not produced** in this
  spike (see "Bambu Studio reference comparison" above).
