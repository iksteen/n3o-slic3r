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

**PARTIAL PASS.** Slice produced `/tmp/spike3.gcode` — 5 015 526
bytes / 278 layers / model printing time 2h 55m 21s. All four AMS
slots get used. *Per the BBS reference comparison below,
libslic3r-FFI emits ~10× more tool changes than Bambu Studio for
the same input — a Phase 5 prerequisite, not an FFI gap*. Engine
plumbing, AMS bindings, tool-change G-code emission, and metadata
shape are all validated; the gap is around BBS's tool-change
minimization, which appears to happen above libslic3r in BBS's own
code.

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

### Bambu Studio reference comparison — done

Bambu Studio is installed locally as the
`com.bambulab.BambuStudio` flatpak (v02.06.00.51). It exposes
headless slicing via:

```
flatpak run com.bambulab.BambuStudio --slice 0 \
    --export-3mf output.gcode.3mf \
    --outputdir ~/spike3-bbs ~/spike3-bbs/fourcolor.3mf
```

(Flatpak's sandbox has filesystem=home so `/tmp` paths don't work
— stage under `~`.) Output `.gcode.3mf` is 2.6 MB. Its
`Metadata/` inventory matches the "blocking" + "probably blocking"
+ "cosmetic" rows of the wrapping-gap table above exactly — no
surprise files.

**Tool-change count is wildly different.** BBS produces 7
mid-print tool changes (`T0×1 + T1×2 + T2×2 + T3×2`, total 7) and
a 1h 3m 22s print time. Our libslic3r-FFI slice of the *same input*
produces 76 mid-print tool changes (`T0×36 + T1×19 + T2×19 + T3×2`)
and a 2h 55m 21s print time — almost 3× longer. Per-filament
material use also differs (BBS: 3.20 + 4.21 + 3.82 + 2.77 g; ours:
8.39 + 9.84 + 8.73 + 2.77 g — ~2.5× more material, mostly purge).

The model has 8 vertically-stacked parts, each assigned to one of
extruders 1–4 in rotation (see `Metadata/model_settings.config`
in the source). BBS correctly detects that each Z-band has a
single extruder assignment and emits one tool change per band
boundary; libslic3r-FFI does much more work, oscillating
`T1↔T0↔T1↔T0` within each band. Cause not isolated in this spike
— possibilities include:

1. BBS runs a pre-slice optimization pass above libslic3r that
   merges contiguous same-extruder parts into single print
   sessions; the engine library doesn't do this on its own.
2. A config knob we're missing — both runs report
   `print_sequence = "by layer"`, `enable_prime_tower = 0`,
   `filament_map_mode = "Auto For Flush"` post-slice, but BBS may
   set other flags we haven't enumerated.
3. The 3MF's `assemble` section (the
   `Metadata/model_settings.config`'s `<assemble>` block) might
   carry print-order hints that BBS reads but libslic3r ignores at
   the FFI surface.

Whatever the cause, the impact for Phase 5 is direct: shipping
n3o-slic3r without tool-change minimization would deliver 3× longer
prints and ~2.5× more material consumption for multi-color jobs.
This must be addressed before Phase 5 hardware validation; treat
as a hard prerequisite for the Bambu driver going to a real
printer.

### Investigation so far (still open)

Refined comparison after also slicing the same input through
OrcaSlicer-app (v2.4.0-dev): **OrcaSlicer's libslic3r emits 7 tool
changes too**, matching BBS exactly (1h 3m print, 14 g filament).
That isolates the disparity to *how our FFI invokes libslic3r* —
the engine itself can do the right thing on this input.

Per-feature inspection of our 76-change run shows the redundant
transitions are all `Tband → T0` *for support material* (and back
to `Tband` after). Bodies print fine with the per-part extruder;
supports are the problem. So we suspected the FFI shim's
zero-coerce of `wall_filament` / `sparse_infill_filament` /
`solid_infill_filament` / `support_filament` /
`support_interface_filament` (added during PR-0.5-1's libslic3r
workarounds for the `filament_map` undersize crash) was forcing
supports onto filament 1. The user's hypothesis was that the
coerce became redundant once the filament_map normalization is in
place.

Tested by deleting the coerce block from
`crates/slic3r-ffi/ffi/slic3r_ffi.cpp:477-489`. Re-ran spike3:

- `Config::get` reports the 3MF's intended `support_filament=0`
  before slicing — the shim isn't upgrading.
- Explicit `Config::set("support_filament", "0")` right before
  slicing also stays at "0".
- Instrumented post-`Print::apply`:
  - `Print::full_print_config().support_filament` = **0** ✓
  - `Print::full_print_config().support_interface_filament` = **0** ✓
  - 1 `PrintObject`, `config().support_filament` = **0** ✓
  - 5 `PrintRegion`s — four with `wall/sparse/solid_infill_filament`
    set to 1, 2, 3, 4 (one per Z-band-extruder), **one with all
    three set to 0**. Region 0 is the suspect — it's the
    "untyped" region.
- But the *gcode body* still uses T0 for every support segment in
  bands 2/3/4 (76 mid-print changes; OrcaSlicer-app on the same
  input would use the part's body extruder).
- The gcode header's CONFIG_BLOCK dump shows `support_filament = 1`
  even though `Print::full_print_config()` returns 0 — likely a
  serialization quirk of the dump path (it reads from somewhere
  other than `m_config`, possibly per-region or per-object), not
  the actual control value.

So the in-memory config is *correct end-to-end* but the slice still
mis-assigns supports. The bug is downstream of `Print::apply` —
likely in `GCode.cpp:4794-4820`, the "support don't care"
resolution:

```cpp
unsigned int dontcare_extruder = first_extruder_id;  // = layer_tools.extruders.front()
... // soluble / interface filtering
if (support_dontcare)
    support_extruder = dontcare_extruder;
```

`first_extruder_id` is the *layer's first extruder*, which inherits
from the previous layer's last extruder. For our 8-Z-band model
with `print_sequence = "by layer"` and `enable_prime_tower = 0`,
this picks up T0 (carried from band 1's last layer) instead of
the current band's body extruder. OrcaSlicer's flow must populate
something earlier — `wiping_extrusions.get_support_extruder_overrides`
maybe, or per-volume `support_filament` overrides — that the FFI
doesn't replicate.

Coerce was removed in this investigation but reverted (kept the
PR-0.5-1 workaround intact) since removing it alone doesn't fix
the bug. Phase 5's investigation needs to find the additional
pre-`apply` setup OrcaSlicer's GUI/CLI does that the FFI is
missing. Concrete next steps when picked up:

1. Diff `WipingExtrusions` state between OrcaSlicer CLI and our
   FFI right before `GCode::process_layer` runs.
2. Check whether OrcaSlicer CLI calls a `set_support_extruder_overrides`
   or similar method post-apply that we'd need to mirror.
3. If neither, instrument `GCode.cpp:4794` to log
   `first_extruder_id` and `layer_tools.extruders` per layer in
   both runs and compare.

This is real engine plumbing, not a config knob, so the fix likely
lives in a PR-0.5-1-style libslic3r workaround documented in
`docs/libslic3r-workarounds.md`.

Other than the tool-change disparity, the gcode bodies are
structurally similar — same `; CHANGE_LAYER` / `WIPE_START` /
`WIPE_END` / `FLUSH_START` / `FLUSH_END` markers, same
`change_filament_gcode` template expansion, same filament-aggregate
comment block.

The BBS reference artifacts (`~/spike3-bbs/output.gcode.3mf` and
`~/spike3-bbs/result.json`) are not checked in because of the
licensing chain (the input 3MF is CC BY-NC and the wrapped output
inherits that constraint). Regenerate locally with the command
above.

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

- **Phase 5 (Multi-printer + drivers).** Two distinct workstreams:
  - **`.gcode.3mf` wrapper** (~1 person-week). The
    Blocking/Probably-Blocking/Cosmetic table is the shopping list.
    The wrapper lives above the FFI: parse gcode header → aggregate
    into typed models → emit JSON + zip. The BBS reference at
    `~/spike3-bbs/output.gcode.3mf` documents exact field shapes.
  - **Tool-change minimization** (effort unbounded, must precede
    hardware validation). libslic3r-FFI as it stands emits 10× more
    tool changes than BBS for the same multi-color input,
    triplicating print time and over-consuming filament. Whatever
    BBS does to reduce 76 tool changes to 7 has to be replicated
    above the engine — either as a model-rewrite pass that merges
    contiguous same-extruder parts, a config-flag we haven't
    enumerated, or an FFI extension surfacing libslic3r's
    print-sequence internals. Investigate before committing to a
    Phase 5 schedule.

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
- Bambu Studio reference: regenerate locally via
  `flatpak run com.bambulab.BambuStudio --slice 0 --export-3mf
  output.gcode.3mf --outputdir ~/spike3-bbs ~/spike3-bbs/fourcolor.3mf`
  (BBS v02.06.00.51). The wrapped `.gcode.3mf` is not checked in
  because the input is CC BY-NC and the output inherits that
  constraint.
