# OrcaSlicer Setting Classification in `libslic3r`

A reference for how OrcaSlicer partitions its introspective configuration
schema into Machine (Printer), Filament, and Process (Print) settings.

> **Scope.** This document describes the FFF code paths. SLA support exists in
> the source tree as legacy code but is no longer surfaced in the UI.

---

## TL;DR

There is no semantic classifier. The classification is a **hand-curated,
hard-coded partitioning** of config-key strings, defined by three static
functions in `src/libslic3r/Preset.cpp`:

| Function                     | Preset type     | Files on disk        |
| ---------------------------- | --------------- | -------------------- |
| `Preset::printer_options()`  | `TYPE_PRINTER`  | `machine/*.json`     |
| `Preset::filament_options()` | `TYPE_FILAMENT` | `filament/*.json`    |
| `Preset::print_options()`    | `TYPE_PRINT`    | `process/*.json`     |

Every `ConfigOptionDef` declared in `PrintConfig.cpp` is routed to exactly one
of these three buckets by virtue of its key string appearing in exactly one of
the three vectors. That's the entire taxonomy.

---

## 1. The Schema Layer (`PrintConfig.{hpp,cpp}`)

Each setting begins life as a `ConfigOptionDef` in
`src/libslic3r/PrintConfig.cpp`. The definition is **type-aware but
category-agnostic** — it records:

- The key string (e.g. `layer_height`, `nozzle_diameter`, `filament_type`)
- Value type (`coFloat`, `coInts`, `coEnum`, `coStrings`, `coBools`, …)
- Default value
- GUI metadata: `label`, `tooltip`, `category`, `sidetext`, min/max
- Mode (Simple / Advanced / Developer)

Nothing in `ConfigOptionDef` itself declares "this is a machine setting." The
`category` field is purely for GUI tab grouping (Quality, Strength, Speed,
etc.), not for preset-file routing.

The schema is initialized once at application startup; see
`PrintConfigDef::PrintConfigDef()` and the surrounding initialization block in
`PrintConfig.cpp` (roughly lines 455–7200 in current `main`).

---

## 2. The Routing Layer (`Preset.cpp`)

`src/libslic3r/Preset.cpp` declares three static option-list functions, each
returning a `std::vector<std::string>` of config keys:

```cpp
static const std::vector<std::string>& Preset::print_options();
static const std::vector<std::string>& Preset::filament_options();
static const std::vector<std::string>& Preset::printer_options();
// (plus sla_print_options() / sla_material_options() — legacy, unused in UI)
```

`PresetBundle` passes the relevant vector to each `PresetCollection` at
construction. From that point on, **a setting "is a machine setting" iff its
key string appears in `printer_options()`** — and analogously for the other
two.

### Implicit policy in each list

Reading what each vector contains, the de-facto rules are:

- **Machine (printer)** — anything physically tied to the hardware:
  - `printer_model`, `printer_variant`, `printer_technology`
  - `nozzle_diameter`, `extruders_count`, `extruder_offset`
  - `gcode_flavor`, `single_extruder_multi_material`
  - `printable_area`, `printable_height`, `bed_exclude_area`
  - `machine_max_acceleration_*`, `machine_max_speed_*`, `machine_max_jerk_*`
  - Start / end / layer-change / tool-change G-code
  - Default retraction values, wipe, Z-hop defaults
  - Thumbnail / network / device-discovery config

- **Filament** — anything that depends on the spool currently mounted:
  - `filament_type`, `filament_vendor`, `filament_colour`, `filament_diameter`
  - `nozzle_temperature`, `nozzle_temperature_initial_layer`
  - `hot_plate_temp`, `bed_temperature` family
  - `filament_flow_ratio`, `filament_max_volumetric_speed`
  - Fan and cooling tables (per-material)
  - Pressure-advance / linear-advance value (per-filament)
  - Per-filament retraction *overrides* (the *defaults* live in the printer)
  - `filament_cost`, `filament_density`

- **Process (print)** — everything geometric or strategic about the slice:
  - `layer_height`, `initial_layer_print_height`
  - `wall_loops`, `wall_generator`, `top_shell_layers`, `bottom_shell_layers`
  - `sparse_infill_density`, `sparse_infill_pattern`, `infill_direction`
  - Support: `enable_support`, `support_type`, `support_style`, interfaces
  - Speeds tied to feature types (outer wall, inner wall, infill, travel, …)
  - Seams, ironing, brim, skirt, raft
  - Arachne thresholds, precision, flow ratios per feature

> **Heuristic:** *if a setting changes when you swap printers but not when you
> swap filament*, it's in `printer_options()`. *If it changes when you swap
> filament but not when you swap models*, it's in `filament_options()`.
> Everything else — the strategy applied to the geometry — is in
> `print_options()`.

---

## 3. The Type Enum and Preset Collections

`src/libslic3r/Preset.hpp` declares:

```cpp
enum Preset::Type {
    TYPE_INVALID,
    TYPE_PRINT,         // process
    TYPE_FILAMENT,
    TYPE_PRINTER,
    TYPE_SLA_PRINT,     // legacy
    TYPE_SLA_MATERIAL,  // legacy
    // ...
};
```

Each `Preset` instance carries its `Type`. `PresetBundle` holds parallel
collections:

- `PresetCollection prints;`     ← `TYPE_PRINT`
- `PresetCollection filaments;`  ← `TYPE_FILAMENT`
- `PrinterPresetCollection printers;`  ← `TYPE_PRINTER`

When a preset JSON file is loaded, the collection it lands in determines its
type, and the key-list from `Preset.cpp` determines which subset of
`DynamicPrintConfig` keys are valid in that file. The function
`Preset::remove_invalid_keys()` literally strips any key that doesn't belong
in the preset's bucket.

### On-disk layout

```
resources/profiles/<vendor>/
├── machine/
│   ├── MyPrinter 0.4 nozzle.json   ← printer_options()
│   └── ...
├── filament/
│   ├── Generic PLA.json            ← filament_options()
│   └── ...
└── process/
    ├── 0.20mm Standard.json        ← print_options()
    └── ...
```

The legacy name `TYPE_PRINT` (rather than `TYPE_PROCESS`) is preserved from
the upstream Slic3r / PrusaSlicer codebase, even though the OrcaSlicer UI
labels these as "Process" presets.

---

## 4. Variant-Indexed Options (the complication)

OrcaSlicer (inheriting from Bambu Studio) supports settings that vary by
**nozzle variant** within a single printer — i.e. a setting can be partly a
printer setting and partly something else.

`PrintConfig.cpp` / `PresetBundle.cpp` define parallel vectors:

- `print_options_with_variant`     — e.g. some speeds, line widths
- `filament_options_with_variant`  — e.g. temperatures, flow ratios
- `printer_options_with_variant_1` and `printer_options_with_variant_2`

For these keys the stored value is an array indexed by nozzle variant rather
than a scalar. At resolution time,
`PresetBundle::update_values_to_printer_extruders()` walks the `filament_map`
config option to map variant slots onto the printer's actual extruders.

So the bucket (`print` vs `filament` vs `printer`) still uniquely identifies
where the setting lives, but **the per-extruder value is resolved against the
printer** at slicing time. This is why changing the printer preset can appear
to change print or filament values even though those values are stored in the
process or filament preset.

---

## 5. Compatibility and Filtering

A printer preset doesn't directly own its compatible filament or process
presets; instead, each filament/process preset declares:

- `compatible_printers` (list of preset names)
- `compatible_printers_condition` (expression evaluated against printer config)
- `compatible_prints` / `compatible_prints_condition` (filament ↔ process)

For filaments there is additional gating on `nozzle_diameter` — a filament
preset is only offered for printers whose nozzle diameter matches.

Relevant entry points:

- `is_compatible_with_print(...)`
- `is_compatible_with_printer(...)`

(both declared in `Preset.hpp`).

---

## 6. Quick Reference: Where to Look

| Question                                          | File                                |
| ------------------------------------------------- | ----------------------------------- |
| What settings exist and what are their types?     | `src/libslic3r/PrintConfig.cpp`     |
| Which bucket does a given setting belong to?      | `src/libslic3r/Preset.cpp` (the three `*_options()` functions) |
| What are the preset types and collections?        | `src/libslic3r/Preset.hpp`          |
| How are presets loaded/saved/merged?              | `src/libslic3r/PresetBundle.cpp`    |
| How do variant-indexed options get resolved?      | `PresetBundle::update_values_to_printer_extruders()` |
| How does slicing know which step to invalidate when a setting changes? | `Print::invalidate_state_by_config_options()` in `src/libslic3r/Print.cpp` |

---

## 7. Practical Implications

- **Adding a new setting** requires three edits at minimum:
  1. Declare its `ConfigOptionDef` in `PrintConfig.cpp`.
  2. Add the key string to **exactly one** of `print_options()`,
     `filament_options()`, `printer_options()` in `Preset.cpp`.
  3. If it should affect a specific slicing step, add the key to
     `Print::invalidate_state_by_config_options()`.

- **Putting a key in the wrong bucket** means it will either be silently
  stripped on load by `Preset::remove_invalid_keys()` or it will appear in
  files where the user can't reach it via the UI tab for that preset type.

- **CLI users** can pass any `PrintConfig` key directly as a flag to
  `orca-slicer`; the bucket only determines which preset file the key is
  *read from*, not whether it's accepted on the command line.

---

## Sources

- OrcaSlicer source tree: `src/libslic3r/Preset.cpp`, `Preset.hpp`,
  `PrintConfig.cpp`, `PrintConfig.hpp`, `PresetBundle.cpp`, `Print.cpp`
- OrcaSlicer Wiki: *Preset and bundle* page
- DeepWiki index of `SoftFever/OrcaSlicer`: *Configuration System* section
