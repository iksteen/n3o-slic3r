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

All five live in `crates/slic3r-ffi/ffi/slic3r_ffi.cpp`. Line numbers
below are stable as of writing.

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

**Fix.** `slic3r_ffi.cpp:445-490` — apply the normalization on a
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

for (const char* key : {"wall_filament", "sparse_infill_filament",
                        "solid_infill_filament", "support_filament",
                        "support_interface_filament"}) {
    if (auto* opt = cfg.option<ConfigOptionInt>(key); opt && opt->value == 0)
        opt->value = 1;
}
```

The temporary-copy approach means the caller's config remains
untouched.

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

If a workaround becomes unnecessary, **leave the code in place with an
updated comment** ("upstream fixed in commit XYZ, kept for older
submodule pin compatibility") until the next stable submodule pin
older than the fix is no longer supported. Don't churn.

## When the shim itself surfaces a new bug

Add a section here using the same shape: symptom, root cause with
file:line, fix location in our shim. Future maintainers will thank
you. The cost of writing a clear failure-mode entry is dwarfed by the
cost of someone else rediscovering the same trap a year later.
