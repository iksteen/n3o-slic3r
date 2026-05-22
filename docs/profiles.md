# Config & Profiles Strategy

> Status: design notes, pre-implementation. Captured during early
> exploration on 2026-05-22. Revisit when we start building the resolver.

## Conclusion (for the impatient)

We adopt a **rule-cascade config format** with selector-based overrides
and CSS-like specificity. The config files live in our own vocabulary
and shape; an **adapter layer in our Rust code** resolves the cascade
against a slice context and emits a flat libslic3r `DynamicPrintConfig`.

libslic3r never sees our config format. Its option vocabulary, vector
indexing, and dispatch quirks (`curr_bed_type` and friends) stay where
they are — fully behind the adapter.

## Why a new format

OrcaSlicer's preset/profile system has accreted from the
Slic3r → PrusaSlicer → Bambu Studio → OrcaSlicer lineage. The shape we
want to escape:

- **737 options, many of which are name-mangled dimensions of one
  logical setting.** "Bed temperature on filament X" is encoded as 14
  separate top-level keys: `cool_plate_temp`, `eng_plate_temp`,
  `hot_plate_temp`, `textured_plate_temp`, `textured_cool_plate_temp`,
  `supertack_plate_temp`, each paired with a `*_initial_layer`
  variant. Plus `curr_bed_type` (an enum) to select which set applies.
- **Hardcoded dispatch in C++** (`PrintConfig.hpp::get_bed_temp_key`,
  a switch over `BedType`). Adding a new plate type touches the enum,
  the switch, 2 new option declarations, profile JSONs for every
  printer that supports it, and start-G-code templates.
- **Inheritance via `inherits` chains in preset JSON**, merged at
  load time by `PresetBundle` (~285 KB of C++ for the merging logic).
  Two-way binding to the GUI is implicit in the preset model.
- **No data-driven dispatch.** "When chamber is enabled, use a
  different temperature" cannot be expressed in the config — it has
  to be done with `{if has_chamber_temperature_control}` blocks in
  G-code template strings, pushing logic into a different language
  inside escaped strings inside a config value.

We want config files where:
- Settings are flat — no schema-level dimension explosion.
- Any setting can be overridden by any context, declaratively.
- Adding a new dimension (chamber-aware, layer-count-aware,
  ambient-aware) is a config change, not a schema change.
- Diffs are small and locally meaningful.
- Form UIs can render values without parsing code.
- Configs are inspectable and shareable.

## How OrcaSlicer handles this today

For reference, since our design intentionally diverges from it.

**Bed temperature, the canonical example:**

```cpp
// PrintConfig.cpp:924-1004 declares (truncated):
def = this->add("supertack_plate_temp", coInts);             // vector indexed by filament
def = this->add("cool_plate_temp", coInts);
def = this->add("textured_cool_plate_temp", coInts);
def = this->add("eng_plate_temp", coInts);
def = this->add("hot_plate_temp", coInts);
def = this->add("textured_plate_temp", coInts);
def = this->add("supertack_plate_temp_initial_layer", coInts);
// ... (and 5 more *_initial_layer variants)
def = this->add("curr_bed_type", coEnum);                    // selects which of the above
```

**Dispatch at slice time** (`PrintConfig.hpp::466`):

```cpp
static std::string get_bed_temp_key(const BedType type) {
    if (type == btSuperTack) return "supertack_plate_temp";
    if (type == btPC)        return "cool_plate_temp";
    if (type == btPCT)       return "textured_cool_plate_temp";
    if (type == btEP)        return "eng_plate_temp";
    if (type == btPEI)       return "hot_plate_temp";
    if (type == btPTE)       return "textured_plate_temp";
    return "";
}
```

**At G-code time** (`GCode.cpp:2999`): the bed temp is computed in C++
from `(curr_bed_type, filament_index)`, then injected into the
placeholder parser as `bed_temperature_initial_layer_single` for the
start_gcode template to reference. The template doesn't know which key
it came from.

**Two dimensions, two completely different mechanisms.** Filament is a
vector index (`opt->get_at(filament_id)`); plate is a key-name
selection (`get_bed_temp_key(BedType)`). Adding new dimensions
requires picking which mechanism to extend — neither generalizes.

## Our model: rule cascades

Each config file is a list of **rules**. A rule has zero or more
`when.*` predicates (the selector) and one or more `set.*` actions
(the settings it applies). At slice time, given a context
(`{filament.type, plate.type, printer.model, nozzle.diameter, ...}`),
the resolver finds all rules whose predicates match, and for each
setting picks the value from the rule with the highest specificity.

### Syntax — TOML

```toml
# Default — always applies (specificity 0)
[[rule]]
set.bed_temp = 50
set.layer_height = 0.2

# Single-condition override (specificity 1)
[[rule]]
when.filament.type = "PLA"
set.bed_temp = 45

[[rule]]
when.filament.type = "PETG"
set.bed_temp = 60

# Plate-specific
[[rule]]
when.plate.type = "PEI"
set.bed_temp = 60

# Plate × filament — most specific, wins both above when both match
[[rule]]
when.filament.type = "PLA"
when.plate.type    = "PEI"
set.bed_temp = 55

[[rule]]
when.filament.type = "PETG"
when.plate.type    = "PEI"
set.bed_temp = 70

[[rule]]
when.filament.type = "PLA"
when.plate.type    = "SuperTack"
set.bed_temp = 45
```

### Syntax — section shorthand (sugar)

For the common single-condition case:

```toml
# These two are equivalent
[filament.type.PLA]
bed_temp = 45
first_layer_bed_temp = 50

[[rule]]
when.filament.type = "PLA"
set.bed_temp = 45
set.first_layer_bed_temp = 50
```

The desugaring: any section name matching `<context_dim>.<value>`
becomes an implicit `when.<context_dim> = <value>` rule, with the
section body as `set.*` entries. Compound conditions require the
explicit `[[rule]]` form.

### Resolution semantics — two phases

Resolution runs in two phases. The first applies the **authored
cascade** (the rules that encode domain knowledge — default, printer,
build_plate, filament). The second applies **absolute overrides** —
the rules generated by the UI when a user changes a value or saves a
profile (user, project, object). Phase 2 wins unconditionally over
phase 1: a project override beats any rule in the authored cascade no
matter how specific the rule was.

The shape mirrors CSS's normal-vs-`!important` model. The authored
cascade is the normal-specificity tier; user, project, and object are
each their own `!important`-style tier on top. Within each tier, the
tier's own rules apply (specificity-and-source-order for the authored
tier; later-source-wins for the override tiers). Between tiers, higher
tiers always win.

#### Phase 1 — authored cascade

Rules loaded in this order: `default → printer → build_plate →
filament[slot]`. The order matters only for same-specificity tie-
breaking; higher specificity always wins regardless of source order.

For each setting:

1. Find all authored rules whose `when.*` predicates match the context.
2. Pick the rule with the highest specificity (count of matched
   `when.*` entries).
3. Tie-break by **source load order** — within a single file, later
   rules win over earlier rules of equal specificity; across files,
   later-loaded files win. A warning fires for equal-specificity ties
   from different files ("loaded `plate-PEI.toml` after
   `filament-PLA.toml`; `bed_temp` from PEI took precedence").
4. If no authored rule matches, the libslic3r-declared default for
   that key is the phase-1 result.

#### Phase 2 — absolute overrides

Three nested tiers, each behaving as if every `set.*` entry carried
CSS's `!important`. Within each tier, files are flat unconditional
overrides (no `when.*` predicates, no `[[rule]]` blocks — just
`set.*` entries authored by the UI).

```
user profile          (tier 1) — saved across projects
   ↓ overridden by
project file          (tier 2) — this project's overrides
   ↓ overridden by
per-object overrides  (tier 3) — for the active object
```

Each tier wins unconditionally over the tier below it and over the
entire authored cascade. So a `project: { set.bed_temp = 50 }` beats
an authored `[[rule]] when.filament=PLA when.plate=PEI → set.bed_temp
= 55` even though the authored rule has specificity 2 and the project
override has specificity 0. The user's explicit click-to-override is
not in competition with rule specificity — that's a different layer.

A higher-specificity rule **never** beats an override in a higher
tier. The tiers don't commingle.

#### Trace output

For every resolved setting, the trace reports:

- The winning value and the tier it came from.
- For phase-2 wins: also the **cascade fallback** — what phase 1
  alone would have resolved to. This is what the "Reset to cascade"
  action would revert to.
- For phase-1 wins: the winning rule's `file:line` and specificity,
  plus the list of matching-but-losing rules with their specificities.

This is the data feeding the "show the source" UX.

### Worked example

Context: `{filament.type = "PLA", plate.type = "PEI"}`.

**Authored cascade only** (no user/project/object overrides):

For `bed_temp`, 4 rules match: default (0), PLA-only (1), PEI-only
(1), PLA+PEI (2). Highest specificity is 2. `bed_temp = 55`.

For `layer_height`, only the default rule matches. `layer_height = 0.2`.

For `first_layer_bed_temp`, only the default. Each setting resolves
independently — the PLA-only rule doesn't touch this key, so PLA-only
contributes nothing here.

**With a project override** that flat-sets `bed_temp = 48`:

Phase 1 still resolves `bed_temp = 55`. Phase 2's project tier wins
unconditionally. Effective value: `48`. Trace reports: tier=`project`,
value=`48`, cascade_fallback=`55` (so the user knows what "Reset to
cascade" would do).

`layer_height` and `first_layer_bed_temp` aren't mentioned in the
project file, so they stay at phase 1's resolution.

**With a per-object override** on the active object setting `bed_temp
= 40`:

Phase 2 tier 3 (object) wins over tier 2 (project). Effective value:
`40`. Trace reports tier=`object`, value=`40`, cascade_fallback=`55`,
and also notes that a project override of `48` exists for non-this-
object slices.

## Translating to libslic3r

Our resolver runs above libslic3r. By the time we call `Print::apply`,
libslic3r sees a perfectly normal flat `DynamicPrintConfig`.

```
┌────────────────────────────────────────────────────┐
│  Config files (our TOML, rule cascade)             │
└────────────────────────────────┬───────────────────┘
                                 │  cascade resolver (our Rust)
                                 ▼  given: context object
┌────────────────────────────────────────────────────┐
│  Resolved logical settings (our vocabulary)        │
│    bed_temp(PLA on PEI) = 55                       │
│    bed_temp(PLA on Cool) = 45                      │
│    layer_height = 0.2                              │
└────────────────────────────────┬───────────────────┘
                                 │  translation manifest (our Rust)
                                 ▼
┌────────────────────────────────────────────────────┐
│  libslic3r DynamicPrintConfig (Orca vocabulary)    │
│    hot_plate_temp = [55, 70]                       │
│    cool_plate_temp = [45, 55]                      │
│    eng_plate_temp = [60, 65]                       │
│    ... (all 14 plate-temp keys populated)          │
│    curr_bed_type = "Textured PEI Plate"            │
│    layer_height = 0.2                              │
└────────────────────────────────┬───────────────────┘
                                 │  Print::apply, process, export_gcode
                                 ▼
                            G-code output
```

### Translation cases

**Identity (the common case).** `set.layer_height = 0.2` → libslic3r
`layer_height = 0.2`. No mapping.

**Dimensional expansion (the dispatch-quirk case).** Settings that
libslic3r expanded across an Orca dimension (bed temp across plate
types, retraction across nozzle-cut state, etc.) need to be populated
for *all* dimension values, because libslic3r's runtime picks one of
them via a separate selector key (`curr_bed_type`, etc.).

For bed temperature on a PEI / PLA+PETG slice:

```
1. Plate selector: set curr_bed_type = "Textured PEI Plate".
2. For each plate type and each filament, resolve the cascade with
   that hypothetical context and emit the result into the
   corresponding Orca key:
     hot_plate_temp   = [resolve(PEI, PLA),       resolve(PEI, PETG)]
     cool_plate_temp  = [resolve(Cool, PLA),      resolve(Cool, PETG)]
     eng_plate_temp   = [resolve(Eng, PLA),       resolve(Eng, PETG)]
     ...
   This means the user *can* slice to a different plate without
   re-resolving — the values are already there. It also matches what
   Orca's profile JSONs do today (they emit values for every plate).
3. "Filament does not support this plate" emits 0 (the libslic3r
   sentinel) for that (filament, plate) pair. Authored either as an
   explicit rule resolving to 0, or as a gap in the cascade where no
   rule matched.
```

### Translation manifest

A small Rust data structure (or TOML file) listing which libslic3r
keys are "dimensional" and how they expand:

```
bed_temp {
    libslic3r_keys_by_plate: {
        Cool        -> cool_plate_temp,
        TexturedCool-> textured_cool_plate_temp,
        Eng         -> eng_plate_temp,
        PEI         -> hot_plate_temp,
        TexturedPEI -> textured_plate_temp,
        SuperTack   -> supertack_plate_temp,
    },
    per_filament_vector: true,  // value at filament_idx
}

first_layer_bed_temp {
    libslic3r_keys_by_plate: { ... },  // *_initial_layer variants
    per_filament_vector: true,
}

nozzle_temperature {
    libslic3r_key: "nozzle_temperature",
    per_filament_vector: true,
}

layer_height {
    libslic3r_key: "layer_height",
    per_filament_vector: false,
}
```

Estimated size: ~50 entries for dimensional cases; identity-map the
rest. Authored once; maintained on libslic3r upgrades when option
keys move.

## Option scope (where each setting can be applied)

libslic3r encodes scope **structurally** — by which static C++ config
class declares each option:

- `PrintObjectConfig` — per-object (set via `ModelObject::config`)
- `PrintRegionConfig` — per-region/volume (set via `ModelVolume::config`),
  inherited from object scope when not overridden
- `PrintConfig` (with parents `MachineEnvelopeConfig` + `GCodeConfig`)
  — project-level, no model-side override path
- `SLAPrintObjectConfig`, `SLAPrintConfig`, `SLAMaterialConfig`,
  `SLAPrinterConfig` — SLA variants

Scope was never surfaced as data by upstream. The FFI now exposes it
as a bitmask on every option def (`slic3r_option_def_t::scope`,
populated in `DefCache::build` from each class's `keys()`). The Rust
wrapper presents it as `OptScope` with predicates:
`is_object()`, `is_region()`, `is_print()`, `is_fff()`, `is_sla()`,
plus SLA-specific variants.

The resolver and adapter use scope for four things:

1. **Rule validation at load time.** A rule with `when.object.id = "X"`
   may only mention `set.*` keys that are object- or region-scoped.
   `set.gcode_flavor = "klipper"` in a per-object rule is a config
   error caught at load, not a silently-ignored value at slice time.

2. **Adapter dispatch.** Resolved values go to different places in the
   `DynamicPrintConfig` tree based on scope:
   - `PRINT` → top-level project config (`m_config` on the Print)
   - `OBJECT` → `ModelObject::config` (per-object override)
   - `REGION` → `ModelVolume::config` (per-region/volume override)
   - SLA scopes follow the same pattern via the SLA classes

3. **UI hints.** A "per-object overrides" panel shows only
   object/region-scoped options. A "printer config" form shows only
   print-scoped options. Scope drives which controls a given form
   should expose.

4. **3MF import sanity check.** Orca's 3MFs split settings between
   project-level (`Metadata/project_settings.config`) and per-object
   (`Metadata/model_settings.config`). A print-scoped key appearing in
   the per-object file indicates a malformed or hand-edited 3MF and
   should be flagged.

An option can belong to multiple scopes — most commonly when an FFF
class and an SLA class both declare the same key (e.g. `layer_height`
is in both `PrintObjectConfig` and `SLAPrintObjectConfig`). The
bitmask handles this naturally.

### Unscoped options

~10% of `print_config_def` (currently 71 of ~737) are unscoped —
`OptScope(0)`. These are not slicing settings; they're preset-bundle
metadata (`compatible_printers`, `compatible_printers_condition`),
host-integration markers (`bbl_use_printhost`), deprecated/dead keys
(`brim_ears` — the real consumer is `brim_type == btEar`), and
similar. They exist in the schema for UI/preset-bundle round-tripping
but never reach slicing. The resolver should treat them as opaque
project-level metadata: round-trippable from imports, never set by
our own rules. A jump in the unscoped count is a drift signal worth
investigating; the FFI's test suite asserts a loose bound on it
(`scoped > 500`, `unscoped < 150`).

## What stays libslic3r-shaped

Three things we do **not** redefine, no matter what the cascade
format looks like:

- **The option vocabulary.** `layer_height`, `wall_filament`,
  `nozzle_diameter`, and the other 734 options libslic3r declares.
  We choose how to *present* them; libslic3r's `DynamicPrintConfig`
  needs them by their canonical names.
- **The semantics of each option.** What `layer_height = 0.2` does
  algorithmically is libslic3r's call. We can't redefine it without
  forking the engine.
- **The dispatch quirks.** `curr_bed_type` must be set or `M190` won't
  reflect the right temperature. `wipe_tower` must be on for
  multi-material toolchange G-code to emit. The shim already
  normalizes filament_map / nozzle_volume_type / wall_filament before
  `Print::apply`; the same kind of normalization extends to anything
  else libslic3r expects to be set up by its GUI/CLI's pre-slice
  hooks.

The cascade and the adapter live entirely *above* this boundary.

## Configs are pure data

Decision: configs contain no code, no expressions, no escape hatches.
Just rules, predicates, and values. The cascade resolves declaratively;
anything that genuinely needs computation either turns into a richer
predicate language (numeric/range conditions, see Open Questions) or
isn't expressible in the profile and lives elsewhere in the system.

(Lua exists in the wider project for G-code post-processing plugins —
that's a separate subsystem with a separate trust and execution model.
Profiles never touch it.)

## Open questions

To settle when implementation starts:

1. **Specificity tie-breaking when condition counts are equal across
   different dimensions.** `when.filament.type = "PLA"` vs
   `when.plate.type = "PEI"` both score 1. Source order resolves it,
   but is that what users expect? Should some dimensions outrank
   others (e.g. plate-type beats filament-type)? Probably not — easier
   to keep all dimensions equal and let users write a more-specific
   rule when they want to disambiguate.

2. **Negative conditions.** `when.filament.type != "ABS"` is useful
   but a footgun (rules become hard to scan). Probably skip for v1.

3. **Numeric / range conditions.** `when.nozzle.diameter >= 0.6` for
   "different speeds on big nozzles." Expands the predicate language
   beyond equality. Useful but skip for v1 unless real cases demand
   it early.

4. **Where rules live.** Per-printer file, per-filament file,
   per-plate file, project file? All of the above, with load order
   defining the tiebreaker. The cascade composes naturally across
   files — no inheritance chains needed.

5. **Migration when libslic3r renames an option.** If
   `wall_filament` becomes `walls_extruder_index`, our translation
   manifest needs an entry; user configs (which only refer to our
   logical setting names) don't need to change. libslic3r has a
   `handle_legacy()` mechanism we could surface for our own logical
   keys too if we ever rename ours.

6. **Validation timing.** Predicate dimensions and setting names
   should be validated at config-load time (typo protection). Scope
   compatibility (`set.gcode_flavor` is only legal in print-scope rules
   — see "Option scope" above) is also load-time. Value types and
   numeric ranges can be validated at resolution time (cheaper if most
   settings never get resolved for a given slice).

7. **Trace tooling.** "Why is bed_temp 55?" should produce a trace:
   "rule at plate-PEI.toml:47 (specificity 2) overrides rule at
   base.toml:12 (specificity 0)." Cheap to build, very valuable for
   debugging shared profiles.

8. **3MF round-tripping.** Orca's 3MFs include the full preset
   config. When we open one, we need to either translate
   Orca-flat-config → our cascade (lossy — we'd lose the dimensional
   structure that wasn't there) or import as a single flat overlay
   ("rule from 3MF" with no `when.*`, all `set.*`). The latter is
   trivial and keeps the imported state preserved as-is.

## Options considered and rejected

For the record:

- **Any programming-language admixture in configs** — whole-file Lua
  scripts, `{ expr = "..." }` escape hatches inside values, embedded
  template expressions. Loses inspectability, form-editability,
  migration story, and shareability — every downloaded profile becomes
  arbitrary code. The cascade handles the use cases we surveyed
  declaratively, so the cost isn't worth it.

- **Layered overlays.** `base.toml` + `pei_overlay.toml when plate=PEI`.
  Simple but doesn't compose well in multiple dimensions — N×M
  overlays for the bed-temp matrix.

- **Dimensional tables in the schema.** Declare that `bed_temp` is
  indexed by `(plate, filament)`. Cleaner than name-mangling, but the
  shape is baked: adding a new dimension requires changing every
  setting's declared shape. Cascade rules separate the predicates
  from the settings, so new dimensions are free.

- **Reactive / dependency-graph configs.** Pretty in theory; very
  hard to debug "why did this value end up as 55."

- **CSS-style selectors in TOML section names**
  (`[filament.type.PLA & plate.type.PEI]`). Required quoted strings
  for compound rules — mixed syntax. The `[[rule]]` form scales
  uniformly. Section shorthand kept for single-condition cases.

## References

- OrcaSlicer's bed-temp implementation:
  - `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp:924-1004`
    (option declarations)
  - `external/OrcaSlicer/src/libslic3r/PrintConfig.hpp:466,489`
    (`get_bed_temp_key`)
  - `external/OrcaSlicer/src/libslic3r/GCode.cpp:2152,2999`
    (slice-time resolution + placeholder parser injection)
- libslic3r config primitives:
  - `DynamicPrintConfig` is just a typed string→value map; see
    `crates/slic3r-ffi/ffi/slic3r_ffi.cpp` for how the shim already
    populates it from arbitrary key/value input.
- Related work: Cura's intent profiles (overlay-style), PrusaSlicer's
  placeholder parser (in-template expression language), Klipper's
  Jinja2 macros (template-time logic).
