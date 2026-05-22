# PR-1-6 — Translation adapter + manifest

Status: ❌ open.

**Scope.** Production adapter living in
`src-tauri/src/core/cascade_adapter/`. Converts resolved logical
settings (from PR-1-3 / PR-1-4) into a `slic3r_ffi::Config` ready
for `Print::apply`. Handles identity mappings (most settings),
dimensional expansion (bed temp across plate types, retraction
across nozzle-cut state, etc.), and dispatch-quirk normalization
beyond what the FFI shim already does (`filament_map`,
`nozzle_volume_type`).

Replaces the stub adapter in `src-tauri/examples/spike1.rs`.

**Acceptance criteria.**

- `pub fn adapt(resolved: &Resolved, ctx: &Context, schema:
  &'static [OptionSchema]) -> Result<Config, AdaptError>` builds
  a `slic3r_ffi::Config` from the resolved logical settings,
  applies dimensional expansion through the translation manifest,
  and applies the dispatch-quirk normalizations. Returns a list
  of dropped logical keys (alongside the Config) for the trace /
  warnings to surface.

- Translation manifest (`core/cascade_adapter/manifest.rs` or
  `manifest.toml`):
  - Identity mapping is the default for any logical key that
    matches a libslic3r key 1:1.
  - Dimensional entries enumerate the ~50 expansion cases
    (bed_temp → 14 `*_plate_temp*` keys driven by `curr_bed_type`;
    per-extruder broadcast for scalar→vector; per-region wall /
    infill filament selector resolution). Format follows
    `docs/profiles.md` "Translation manifest" sketch.
  - Drop list: the 67 Bambu-side and 13 Prusa-side
    OrcaSlicer-only keys discovered by PR-0.5-1 and PR-0.5-2,
    plus the 5 Orca typos (`detraction_speed`,
    `inital_layer_height`, `nozzle_temperature_intial_layer`,
    `tree_support_bramch_diameter_angle`, `wall_infill_order`)
    silently remapped to their correct spellings.

- Dispatch-quirk normalizations: `curr_bed_type` set so libslic3r's
  bed-temp selector picks the right vector entry; `wipe_tower`
  normalized for multi-material toolchange G-code emission;
  `filament_map` / `nozzle_volume_type` / per-region filament
  selectors already handled by the FFI shim, but if logical
  cascade values diverge from the shim's normalization, the
  adapter wins.

- Unscoped options (the ~71 keys with no scope bitmask, e.g.
  `application`, `slicer_version`, `generator`) are passed through
  as opaque project metadata: read from the source 3MF if
  present, never set by our cascade rules, written verbatim to
  the output config.

- Tests:
  - Apply the resolved A1 mini cascade through the adapter; the
    resulting `Config` slices `OrcaCube_v2.3mf` successfully.
    Mirrors spike1's exit smoke, but driven by the production
    adapter instead of the stub.
  - Dimensional expansion: a single logical `bed_temp = 65`
    resolved against `plate.type = "Textured PEI"` writes all 14
    `*_plate_temp*` keys to their per-plate values (each plate
    type resolved against its own hypothetical context) and sets
    `curr_bed_type = "Textured PEI Plate"`.
  - Drop list: the 67 Bambu-only keys are dropped silently,
    confirmed by no `UnknownKey` errors from `Config::set` and
    no warnings logged at `WARN+` level (debug-level info is
    fine).
  - Typo remap: a resolved value at logical key
    `nozzle_temperature_intial_layer` is silently rewritten to
    `nozzle_temperature_initial_layer` before `Config::set`.
  - Unscoped roundtrip: a 3MF carrying `application =
    "BambuStudio-02.06.00.51"` round-trips through load + adapt
    + slice with the original value preserved in the output gcode
    header.

**Effort.** ~5 days. Enumerating the manifest and getting
dimensional expansion right against real device profiles is the
bulk of the work.

**Dependencies.** PR-1-1 (schema drives the manifest's
identity-fallback), PR-1-3 (resolver provides the input).

**Out of scope.** Tool-change minimization (PR-1-12 owns
investigation; whatever fix lands there integrates with the
adapter later). Per-object overrides (Phase 3+). User-facing
warnings about drop-listed keys — log only.
