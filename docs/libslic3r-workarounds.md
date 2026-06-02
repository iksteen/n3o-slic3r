# libslic3r workarounds applied by the FFI shim

> Status: as of OrcaSlicer submodule pin `956fcea7e2`. Re-verify before
> bumping the submodule; some of these may have been fixed upstream.

OrcaSlicer's `libslic3r` was designed to be driven by its GUI (and to
a lesser extent its CLI). Driving it headlessly through our FFI surfaces
five real quirks that the shim compensates for. Each was found
empirically — symptoms ranged from silent empty models to
segmentation faults. **Don't remove these workarounds without
verifying upstream has fixed the root cause.** If a future OrcaSlicer
bump silently changes the behavior they rely on, slicing breaks in
non-obvious ways.

All of these but §9 live in `crates/slic3r-ffi/ffi/slic3r_ffi.cpp`; §9 is
a Rust-side adapter guard on what we *send* the engine rather than a patch
to the engine's behavior. Line numbers below are stable as of writing.

---

## 1. `temporary_dir` defaults to filesystem root

**Symptom.** Loading any 3MF returns "The supplied file couldn't be
read because it's empty," even on a valid multi-megabyte 3MF.

**Root cause.** `Slic3r::temporary_dir()` defaults to
`/orcaslicer_model` when not set. The BBS 3MF importer writes a
working-copy backup of every loaded 3MF into `temporary_dir()/<plate
id>/...` *before* parsing geometry. On a non-root user this fails
permission-denied. The loader catches the failure, bails out with no
objects populated, and the "empty file" message fires later in
`Model.cpp:355` as a catch-all.

Upstream's CLI sets `temporary_dir` via `wxFileName::GetTempDir()`
(OrcaSlicer.cpp:1255). The GUI sets it during app startup.

**Fix.** `slic3r_ffi.cpp:267` — call `Slic3r::set_temporary_dir(
std::filesystem::temp_directory_path().string())` in `slic3r_init`.
C++17's filesystem equivalent gives us the same default
(`$TMPDIR`/`/tmp`/etc.) without depending on wx.

---

## 2. `Model::read_from_file` discards 3MF objects without `LoadStrategy::LoadModel`

**Symptom.** 3MF files appear to load (no error from libslic3r), but
the resulting `Model` has zero objects. Slicing then hits the
"couldn't be read because it's empty" message at `Model.cpp:355`.

**Root cause.** `Model::read_from_file`'s default options parameter is
`LoadStrategy::AddDefaultInstances` — which does *not* include
`LoadStrategy::LoadModel`. Without `LoadModel`, the BBS 3MF importer's
`_handle_end_object` (`bbs_3mf.cpp:3611`) deletes each parsed object
instead of attaching it:

```cpp
bool _BBS_3MF_Importer::_handle_end_object() {
    if (!m_load_model) {
        delete m_curr_object;
        m_curr_object = nullptr;
        return true;
    }
    ...
}
```

The flag also gates the "build/restore model state" branch
(`bbs_3mf.cpp:1374`). STL/OBJ/STEP loaders ignore it harmlessly.

**Fix.** `slic3r_ffi.cpp:405` — pass
`LoadStrategy::LoadModel | LoadStrategy::LoadConfig |
LoadStrategy::AddDefaultInstances` to `Model::read_from_file`
unconditionally. `LoadConfig` is also needed to pull in the 3MF's
embedded `Metadata/project_settings.config`.

---

## 3. `Print::is_BBL_printer()` is an uninitialized manual flag

**Symptom.** Slicing Bambu A1 mini / X1C / P1S 3MFs fails validation
with "Relative extruder addressing requires resetting the extruder
position at each layer to prevent loss of floating point accuracy.
Add `G92 E0` to layer_gcode."

**Root cause.** `Print::m_isBBLPrinter` is declared at `Print.hpp:1143`
*without an initializer*:

```cpp
bool m_isBBLPrinter;
```

So its initial value is whatever uninitialized stack memory contains
(typically `false`, but undefined). The flag is exposed via
`is_BBL_printer()` (`Print.hpp:1070`) and consulted by validators that
need to allow Bambu's relative-extrusion + no-G92-per-layer convention
(`Print.cpp:1679`).

Upstream's CLI sets it explicitly after `apply()` by checking the
`printer_model` prefix (`OrcaSlicer.cpp:5985`). The GUI sets it from
the active preset bundle (`BackgroundSlicingProcess.cpp:199`).

**Fix.** `slic3r_ffi.cpp:500` — after `print.apply(model, config)`,
inspect the config's `printer_model` string and set:

```cpp
print.is_BBL_printer() = (printer_model.compare(0, 9, "Bambu Lab") == 0);
```

---

## 4. Pre-`apply` config normalization

**Symptom.** `Print::process()` segfaults deep in tool-ordering with a
backtrace through `calc_filament_change_info_by_toolorder` or
`check_filament_printable_after_group`
(`ToolOrdering.cpp:67,1282`). The crash dereferences a sentinel
`(unsigned int)-1` (0xFFFFFFFF) that should never have been an
extruder index.

**Root cause.** Several config fields need to be sized to the
printer's geometry *before* `Print::apply`. Upstream's CLI does this
between loading and apply (`OrcaSlicer.cpp:5953-5964`):

- `filament_map` must have one entry per filament. For single-extruder
  printers, every entry should be `1` (1-based extruder index).
- `nozzle_volume_type` must have one entry per extruder, defaulting to
  `nvtStandard`.
- Per-region filament selectors (`wall_filament`, `sparse_infill_filament`,
  `solid_infill_filament`, `support_filament`,
  `support_interface_filament`) carry `0` in 3MFs as a "use default"
  sentinel. The GUI resolves these via `PartPlate` state we don't
  replicate. Without the resolution, `ToolOrdering::collect_extruders`
  emits the `0` directly, then `handle_dontcare_extruder` fails to
  resolve it (no positive extruder found in any layer), and downstream
  consumers dereference what becomes the `(unsigned)-1` sentinel.

**Fix.** `slic3r_ffi.cpp:445-475` — apply the normalization on a
temporary copy of the caller's config before `Print::apply`:

```cpp
DynamicPrintConfig cfg = config->cfg;
const size_t extruder_count = cfg.option<...>("nozzle_diameter")->values.size();
const size_t filament_count = cfg.option<...>("filament_diameter")->values.size();

auto& filament_map = cfg.option<ConfigOptionInts>("filament_map", true)->values;
if (filament_map.size() < filament_count) filament_map.resize(filament_count, 1);
if (extruder_count == 1) for (size_t i = 0; i < filament_count; ++i) filament_map[i] = 1;

if (!cfg.has("nozzle_volume_type"))
    cfg.option<ConfigOptionEnumsGeneric>("nozzle_volume_type", true)
        ->values.resize(extruder_count, nvtStandard);
```

The temporary-copy approach means the caller's config remains
untouched.

**Update during PR-0.5-3 — and walked back during PR-3-11.** An
additional zero-coerce block for the `wall_filament` /
`sparse_infill_filament` / `solid_infill_filament` /
`support_filament` / `support_interface_filament` selectors was
originally part of this workaround. PR-0.5-3 removed it, claiming
the coerce was redundant once the `filament_map` normalization
above was in place ("all 16 api tests green; `Config::get` reports
the source's 0 end-to-end through `Print::full_print_config` and
`PrintObject::config`"). The premise was that since the *in-memory*
config carries 0 cleanly, the engine must handle the sentinel
internally.

The premise was wrong. The api tests + spike1/spike2 don't exercise
the multi-color (4 filament) path through `ToolOrdering::
sort_and_build_data` →
`ToolOrdering::reorder_extruders_for_minimum_flush_volume` →
`Slic3r::check_filament_printable_after_group`. The fourcolor.3mf
case in `examples/spike3/` does — and **without** the coerce, that
path SIGSEGVs deterministically before producing any output.

`git bisect` between the 2026-05-22 LKG (06261cc, PR-0.5-3 spike3
run) and HEAD pinpointed commit `1bcf46d` ("PR-0.5-3: document
tool-change disparity investigation + CI memory fix") as the
introducer; restoring the coerce restores the slice.

The coerce stays in — but the **shape** of the coerce matters,
and PR-3-11 corrected the shape twice in two passes:

**Pass 1 (intermediate).** Restored the pre-1bcf46d block in
its original form: hardcoded-1 for all five selectors
(`wall_filament`, `sparse_infill_filament`,
`solid_infill_filament`, `support_filament`,
`support_interface_filament`). Fixed the SIGSEGV but kept the
76-vs-7 tool-change disparity — actually CAUSED it, since
pinning support_filament to any non-zero value suppresses
libslic3r's per-layer support-extruder routing.

**Pass 2 (final).** Split the resolution by axis:

- **Per-REGION selectors** (`wall_filament`,
  `sparse_infill_filament`, `solid_infill_filament`): MUST be
  non-zero or `handle_dontcare_extruder(-1)` inside
  `Print::process` can't find a non-zero extruder to promote
  and the sentinel persists, segfaulting downstream. Resolve
  per-object using each object's `<metadata key="extruder">`
  hint (lifted by the BBS importer into
  `ModelObject::config["extruder"]`). Falls back to the
  common-across-objects default at the print level, or `1` if
  objects disagree.
- **Per-OBJECT support selectors** (`support_filament`,
  `support_interface_filament`): LEAVE at 0 (dontcare). The
  per-layer routing at `GCode.cpp:4794-4820` picks
  `first_extruder_id = layer_tools.extruders.front()` for each
  layer — for the fourcolor stacked case, only one band is
  active per layer, so supports inherit that band's body
  extruder and produce the right number of tool changes (7,
  matching Orca/BBS). Coercing support_filament to any non-zero
  value breaks this.

Confirmed empirically on `examples/spike3/fourcolor.3mf` with
the embedded BBS config: 7 mid-print tool changes
(`T0×1 + T1×2 + T2×2 + T3×2`), 1h 6m 23s estimated print time,
matching the BBS reference within slicing-arithmetic precision.

Loud `// Per-REGION… Per-OBJECT…` comment block in the FFI
shim cross-links both halves of the resolution + the SIGSEGV /
tool-change history so neither side can be quietly undone.

---

## 5. `coEnums` defaults can't be serialized via the option's own `serialize()`

**Symptom.** During `DefCache::build`, calling
`d.default_value->serialize()` for any `coEnums` (vector-of-enums)
option segfaults with a backtrace through
`ConfigOptionEnumsGenericTempl::serialize_single_value` at
`Config.hpp:2190`, dereferencing a null `keys_map`.

**Root cause.** The serializer reads `this->keys_map` — a member of
the option object itself, not the def:

```cpp
void serialize_single_value(std::ostringstream& ss, const int v) const {
    if (v == nil_value()) { ... }
    else {
        for (const auto& kvp : *this->keys_map)   // ← null when set via set_default_value
            if (kvp.second == v) ss << kvp.first;
    }
}
```

`set_default_value(new ConfigOptionEnumsGeneric{...})` clones the
option without propagating the def's `enum_keys_map` pointer. The def
has the mapping in `ConfigOptionDef::enum_keys_map`; the serializer
just doesn't consult it.

9 options are affected: `overhang_fan_threshold`, `nozzle_type`,
`z_hop_types`, `retract_lift_enforce`, `extruder_type`,
`nozzle_volume_type`, `default_nozzle_volume_type`,
`filament_z_hop_types`, `filament_retract_lift_enforce`.

**Fix.** `slic3r_ffi.cpp:90` — `serialize_coenums_default(d)` mirrors
the standard serializer but pulls the reverse-lookup map from the def
instead of the option:

```cpp
std::string serialize_coenums_default(const ConfigOptionDef& d) {
    if (!d.default_value || !d.enum_keys_map) return {};
    const auto* opt = dynamic_cast<const ConfigOptionVector<int>*>(d.default_value.get());
    if (!opt) return {};
    std::string out;
    bool first = true;
    for (int v : opt->values) {
        if (!first) out += ',';
        first = false;
        for (const auto& kvp : *d.enum_keys_map) {
            if (kvp.second == v) { out += kvp.first; break; }
        }
    }
    return out;
}
```

`DefCache::build` (line 173) dispatches to this for `coEnums` and to
the standard `d.default_value->serialize()` for everything else.

---

## 6. `Print::m_origin` (plate origin) is uninitialized

**Symptom.** A multi-material slice (≥2 filaments, so the clearance check
has a non-trivial exclusion polygon to test) fails *validation* — before
any slicing — with ClipperLib's "Coordinate outside allowed range",
thrown from `Print::validate()` → `layered_print_cleareance_valid`
(`Print.cpp:959`). It's **heap/binary-dependent**: an unrelated change to
the shim can flip a previously-passing fixture (e.g. `fourcolor.3mf`)
into failure, because the bad value is uninitialized memory.

**Root cause.** `Print::m_origin` (the per-plate origin) is declared at
`Print.hpp:1173` *without an initializer*:

```cpp
Vec3d   m_origin;
```

Eigen's default constructor leaves it uninitialized, so it holds garbage
(observed: denormal doubles like `{2e-320, 6.9e-310, …}`). The clearance
check reads it via `get_plate_origin()` and translates the bed-exclusion
polygon by `scale_(m_origin.x()), scale_(m_origin.y())`
(`Print.cpp:937`). When the garbage is large, the translated polygon's
coordinates exceed ClipperLib's `hiRange` (`0x3FFFFFFFFFFFFFFF`) and the
range check throws. The origin is normally set by the GUI's PartPlate
(`set_plate_origin`); a headless slice never touches it.

**Fix.** `slic3r_ffi.cpp` — after `print.apply(model, config)`, pin the
origin to zero (the plate sits at the bed origin in our single-plate
headless model):

```cpp
print.set_plate_origin(Vec3d(0.0, 0.0, 0.0));
```

Same class of bug as workaround 3 (`is_BBL_printer()`): an uninitialized
`Print` member the GUI would otherwise set. A second, distinct
uninitialized-read on the skirt path (`WipeTowerData::height`) is
workaround 7 below.

---

## 7. `WipeTowerData::height` is read uninitialized on the BBL (Type1) tower path

**Symptom.** A BBL (Bambu) multi-material slice with a prime tower
*intermittently* fails with ClipperLib "Coordinate outside allowed range"
during `Print::_make_skirt` (Print.cpp). Heap-layout dependent: the same
input slices fine on most builds and throws on others — any unrelated
change to the shim can flip it.

**Root cause.** A latent upstream UB bug — **not** a config divergence. A
real OrcaSlicer A1 inherits the exact same values we do
(`fdm_process_common.json`: `skirt_height=1`, `skirt_loops=0`; engine
default `wipe_tower_cone_angle=30`). The chain:

- `Print::has_skirt()` returns `skirt_height > 0` — *not* `skirt_loops` —
  so with the default `skirt_height=1`, `_make_skirt` runs even with zero
  skirt loops.
- `_make_skirt` builds a convex hull over
  `first_layer_wipe_tower_corners()`, whose stabilization-cone math is
  **not** gated by wall type: it computes
  `R = tan(cone_angle/2) * m_wipe_tower_data.height` for *every* tower.
- BBL printers are forced onto the Type1 (old, rectangular) wipe tower
  (`Print::wipe_tower_type()` returns Type1 when `is_BBL_printer()`), and
  **only the Type2/rib branch ever assigns `m_wipe_tower_data.height`**
  (`Print.cpp:3449`). The Type1 path leaves it as an uninitialized
  `float height;`.
- `cone_angle=30` × garbage `height` → an infinite cone radius → an
  `INT64_MIN` corner → the range check throws.

It "works for millions" only because reading uninitialized memory usually
lands on a small/benign value (a small cone, no overflow). Genuine UB
masked by lucky memory; OrcaSlicer's A1 runs this exact path.

**Fix.** `slic3r_ffi.cpp` — in the pre-`apply` config normalization, pin
`wipe_tower_cone_angle = 0` when the printer is BBL (a Type1 tower has no
stabilization cone anyway). `R = tan(0) * height = 0` regardless of the
unset `height`, so the corner is deterministic — strictly better than
upstream's reliance on benign garbage. Non-BBL printers (e.g. the
Snapmaker U1, Type2/rib) do set `height` and want a real cone, so their
value is left untouched.

An equivalent root fix is a one-line `= 0` initializer on
`WipeTowerData::height` in the submodule (and arguably gating the cone on
the rib wall type) — both upstreamable — but we keep workarounds shim-side
per this doc's model.

---

## 8. `Print::validate()` null-derefs its `warning` out-param

**Symptom.** A multi-material slice hard-crashes (SIGSEGV, no Rust panic,
the whole app exits) *before* `process()` runs. Reproduces deterministically
on some plates and not others — looks printer-specific (e.g. BBL crashes, a
Snapmaker U1 plate doesn't) but is actually driven by the *filaments* on the
plate, not the printer.

**Root cause.** Not UB, not heap-dependent — a plain null-pointer deref.
`Print::validate(StringObjectException *warning = nullptr, …)` reports
*non-fatal validation warnings* by writing through `warning`, and ~20 of
those sites deref it **unconditionally** (Print.cpp:979–1891). The headless
entry called `print.validate()` with no args, so `warning` was `nullptr`;
whenever a plate trips one of those warning conditions, `warning->string = …`
writes through null and crashes. The condition that surfaced it was
`!has_same_shrinkage_compensations()` (Print.cpp:1890) — fires when the
filaments on a multi-material plate have mismatched shrinkage-compensation
values, which is why it tracked the filament set rather than the printer.

The GUI never hits this: it always passes a real `StringObjectException*`
to `validate()` to surface warnings to the user, so the writes have a valid
target. This is the same class as §3/§6 — *invocation* setup the GUI does
that the headless path skipped, not an engine defect we provoke.

**Fix.** `slic3r_ffi.cpp` — pass a local `StringObjectException` sink to
`validate()` (`print.validate(&validation_warning)`) and discard it (we have
no warning UI). The *returned* `StringObjectException` remains the fatal
error we gate on; the sink just absorbs the advisory writes. One sink covers
all ~20 sites — none can deref null. Mirrors the GUI exactly.

---

## 9. Short per-filament vectors crash MMU segmentation (guarded Rust-side)

**Symptom.** A multi-material slice whose `filament_colour` (or any other
per-filament vector) carries *fewer* elements than the printer has filaments
segfaults inside multi-material segmentation — `apply_mm_segmentation`
(`PrintObjectSlice.cpp:874`) → `get_extents`, reached from
`multi_material_segmentation_by_painting` (`MultiMaterialSegmentation.cpp:2198`).
Surfaced first when a P1S project (2 filaments) was rebound onto the U1
(4 toolheads) and its imported `filament_colour` override stayed length-2
while the rest of the filament vectors fanned to 4.

**Root cause.** `apply_mm_segmentation` sets `num_extruders =
filament_diameter.size()` and indexes `segmentation[layer][extruder_id]`,
but the painting sizes the inner vector to `num_facets_states - 1 =
filament_colour.size()`. When `filament_colour.size() < filament_diameter.size()`
the per-extruder index runs past the end → OOB read → segfault. libslic3r
assumes every per-filament vector is exactly `num_extruders` long (the GUI
guarantees this by construction); a short one is undefined behavior. Unlike
the broadcast-on-`get_at` clamp libslic3r applies elsewhere, the segmentation
path indexes raw, so the lengths must match exactly.

**Fix (Rust-side, not the shim).** Unlike workarounds 1–8, this guard lives
in our adapter, not the FFI shim — it polices what we *send* libslic3r rather
than patching libslic3r itself.
`src-tauri/src/core/cascade_adapter/adapter.rs::check_filament_vector_lengths`
rejects the slice — with the offending keys + the expected count — when any
filament-bucket vector is shorter than `filament_diameter`. Filament-bucket
vector keys are identified from the schema's `bucket` + `is_vector` signals,
not a curated list. The normal cascade fans every filament vector to the slot
count, so this only fires on an already-inconsistent config (a stray override,
a composer gap): it *surfaces* the error rather than padding the vector to
length, which would feed libslic3r guessed per-filament print parameters (a
wrong hotend temperature is dangerous, not merely cosmetic). Longer-than-
`num_extruders` vectors are harmless (libslic3r ignores the surplus) and pass
through. Element counting is cstyle-aware
(`profile_library::split_for_key`, the inverse of `join_for_key`) so the
`;`-comments embedded in `filament_start_gcode` / `_end_gcode` string vectors
aren't miscounted into a spurious failure.

This complements workaround 4 (the shim-side `filament_map` /
`nozzle_volume_type` sizing): §4 sizes vectors *up* inside the shim before
`apply` for the dimensions the GUI would have sized; §9 *rejects* a
genuinely-inconsistent short filament vector before it ever reaches the
engine, instead of guessing the missing values.

---

## When bumping the OrcaSlicer submodule

Re-verify each workaround:

1. Does `temporary_dir()` still default to `/orcaslicer_model`? If
   upstream fixed it (e.g. set a sensible default in
   `libslic3r_static_initializer`), our `set_temporary_dir` becomes
   redundant but harmless.

2. Does `Model::read_from_file`'s default options still exclude
   `LoadModel`? Check
   `external/OrcaSlicer/src/libslic3r/Model.cpp:read_from_file`. If
   the default changed, our explicit `LoadStrategy::LoadModel | ...`
   is still correct.

3. Does `Print::m_isBBLPrinter` still lack an initializer? Check
   `Print.hpp:1143`. If they fixed it (`bool m_isBBLPrinter = false;`),
   our explicit set is still needed because the flag's *semantics*
   depend on the printer profile.

4. Did upstream add a "headless slice setup" API on `Print` that
   normalizes filament_map / nozzle_volume_type / wall_filament for
   us? Look for new public methods on `Print` near apply().
   `print->set_check_multi_filaments_compatibility(...)`,
   `set_filament_maps(...)`, etc. — if such a helper exists and is
   maintained for non-GUI use, prefer it over our manual normalization.

5. Does the coEnums serializer still dereference a null `keys_map`?
   The fix upstream would be to pass `enum_keys_map` through
   `set_default_value`. Check `ConfigOptionEnumsGeneric::set_default_value`
   if it exists, or the def's `set_default_value` overload.

6. Does MMU segmentation still index per-filament vectors raw against
   `filament_diameter.size()` (§9)? Check `apply_mm_segmentation`
   (`PrintObjectSlice.cpp`) and `multi_material_segmentation_by_painting`
   (`MultiMaterialSegmentation.cpp`). If upstream added a clamp/broadcast on
   the short-vector path, our adapter guard becomes belt-and-suspenders but
   stays correct (a short filament vector is still a config inconsistency
   worth surfacing).

If a workaround becomes unnecessary, **leave the code in place with an
updated comment** ("upstream fixed in commit XYZ, kept for older
submodule pin compatibility") until the next stable submodule pin
older than the fix is no longer supported. Don't churn.

## When the shim itself surfaces a new bug

Add a section here using the same shape: symptom, root cause with
file:line, fix location in our shim. Future maintainers will thank
you. The cost of writing a clear failure-mode entry is dwarfed by the
cost of someone else rediscovering the same trap a year later.

## Upstream bugs observed (not worked around)

Bugs found in libslic3r that we've **not** patched — either because
the impact is harmless to our use case, the fix is unclear, or it
needs to land upstream rather than as a local diff. Recorded so we
can surface them on submodule bumps and to file upstream PRs when
time allows.

### GCode emission writes uninitialized bytes to disk

**Symptom**: valgrind reports `Syscall param write(buf) points to
uninitialised byte(s)` from `Slic3r::GCode::GCodeOutputStream::write`
at `GCode.cpp:6181`, fired by `fwrite` inside the gcode export
pipeline. Triggered during the bambi multi-color slice in
`phase_s_smoke`; expected to fire on any slice that exercises the
same gcode-writing path.

**Trace** (compact):

```
write (write.c:26)
  ← fwrite (iofwrite.c:44)
  ← Slic3r::GCode::GCodeOutputStream::write (GCode.cpp:6181)
  ← Slic3r::GCode::GCodeOutputStream::writeln (GCode.cpp:6190)
  ← Slic3r::GCode::_do_export (GCode.cpp:3101)
  ← Slic3r::GCode::do_export (GCode.cpp:2093)
  ← Slic3r::Print::export_gcode (Print.cpp:2586)
```

**Root cause**: somewhere upstream of `writeln`, a `std::string`
emitted into the gcode body carries uninit bytes. Not traced to the
specific call site (would need source instrumentation of
`_do_export`'s ~hundred `writeln` calls).

**Why not worked around**: the bytes that leak are part of gcode
output the printer firmware tolerates (typically inside a comment or
a numeric field that gets re-tokenized). No functional impact on
sliced output we've seen. Fixing requires source-level changes inside
libslic3r — too invasive for a local patch, better as an upstream
issue / PR.

**On submodule bump**: re-run valgrind on `phase_s_smoke` and confirm
whether this is still present. If upstream fixed it, retire this
entry.
