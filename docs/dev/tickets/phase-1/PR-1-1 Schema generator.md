# PR-1-1 — Schema generator (libslic3r → typed Rust)

Status: ✅ done. `src-tauri/src/core/schema/mod.rs` ships an `OptionSchema` cache built from `slic3r_ffi::option_defs()`, with `DimensionalKind::BedTempPerPlate` tagging the 12 per-plate-temp keys. 7 unit tests green; full workspace at 23 passing.

**Scope.** Read libslic3r's option definitions via the existing
FFI introspection (`option_defs()`, `OptionDef`) and emit a typed
Rust schema that the resolver, adapter, and UI all consume. One
struct per option carrying name, type, scope bitmask, per-extruder
vs scalar shape, dimensional-expansion metadata, default value,
enum variants where applicable, and the OrcaSlicer category /
label for future UI use.

Drives three downstream pieces:

- **Load-time validation** (PR-1-2): predicate dimensions and `set.*`
  keys must exist in the schema; misspellings raise a file:line
  error.
- **Adapter type safety** (PR-1-6): scalar-vs-vector keys serialized
  correctly; dimensional keys (bed_temp etc.) routed through
  expansion logic.
- **Context-state shape** (PR-1-7): printer profile structures
  reference schema entries when declaring which options are
  printer-fixed vs per-context.

**Acceptance criteria.**

- `core/schema/` module (new) exposes
  ```rust
  pub struct OptionSchema {
      pub key: String,
      pub ty: OptType,              // already in slic3r-ffi
      pub scope: OptScope,          // already in slic3r-ffi
      pub is_vector: bool,          // per-extruder vs scalar
      pub dimensional: Option<DimensionalKind>,
      pub label: Option<String>,
      pub category: Option<String>,
      pub enum_values: Vec<(String, String)>,  // key → label
      pub default_serialized: Option<String>,
  }
  pub fn load_schema() -> &'static [OptionSchema];   // cached
  pub fn schema_by_key(key: &str) -> Option<&'static OptionSchema>;
  ```
  Builds once at app startup from `slic3r_ffi::option_defs()`.

- `DimensionalKind` enum enumerates the known dimensional axes:
  `BedTempPerPlate`, `RetractionPerExtruder`, … (start with the
  list from `docs/dev/profiles.md` "Translation cases"; extend as
  PR-1-6 surfaces more).

- `is_vector` is set for any libslic3r option whose `OptType`
  is plural (`Floats`, `Ints`, `Bools`, `Strings`, `Percents`,
  `Enums`, `Points`, `Bools`). Already discoverable via the FFI;
  this is a derived bit on `OptionSchema`.

- A small test (`#[test] schema_has_layer_height_and_bed_types`)
  asserts the canonical options come through with correct shapes:
  `layer_height` scalar Float, `nozzle_diameter` vector Float,
  `curr_bed_type` Enum with at least Cool/PEI/SuperTack values,
  `hot_plate_temp` vector Float marked `BedTempPerPlate`.

- Schema covers all 737 options reported by the introspect
  example. A second test asserts the count matches
  `slic3r_ffi::option_defs().len()`.

**Effort.** ~2 days. Most of the work is enumerating
`DimensionalKind` variants and binding them to specific libslic3r
keys — that's the part that requires reading libslic3r source to
get right.

**Dependencies.** Phase 0 complete. PR-0.5-1 confirmed the FFI
introspection surfaces what we need.

**Out of scope.** Per-printer overrides — those live in the
context-state (PR-1-7), not the schema. UI rendering hints
(widget kind, min/max validation) — Phase 4. Dimensional
expansion *logic* — that's the adapter (PR-1-6); this ticket only
declares which keys are dimensional.
