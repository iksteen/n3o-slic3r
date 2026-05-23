# PR-4-1 — Settings backend introspection enrichment

Status: ✅ shipped — `core/schema/capability.rs` ships the `CapabilityPredicate` enum (5 variants: `RequiresMultiSlot` / `RequiresToolchanger` / `RequiresPurgeTower` / `RequiresBblPrinter` / `RequiresChamberHeater`) + a hand-curated `capability_for_key` table mapping ~17 libslic3r keys to their gating predicate. Each variant evaluates against `PrinterProfile` via `satisfied_by`. `RequiresChamberHeater` is a no-op placeholder until `PrinterProfile` carries a `has_chamber_heater` field (PR-4-5's call). `core/cascade::OptionSummary` gained `mode` (`OptMode` enum serialized lowercase), `scope` (`OptScopeFlags` struct of bools), `capability` (`Option<CapabilityPredicate>`), and `tooltip` (`Option<String>`). New Tauri command `slicer_options_for_printer(printer, filter)` returns the same shape plus a pre-evaluated `hidden: bool` so per-row visibility is a single field read in the panel render. 10 tests (6 capability unit + 4 cascade integration); A1 mini hides `extruder_clearance_radius` and shows `flush_volumes_matrix`; synthetic 2-toolhead inverts; full sweep < 500 ms in debug for ~600+ options.

**Scope.** The existing `slicer_options` Tauri command (`core/
cascade/mod.rs:82`) returns `OptionSummary { key, ty, label,
category, default_value }` from `slic3r_ffi::option_defs()`. Phase 4
needs three more dimensions on top of this for the UI to drive its
filters and per-row affordances:

- **Mode** (Simple / Advanced / Expert / Develop) — for FR-UI-2's
  mode filter. Source: `ConfigOptionDef::mode`. Already surfaced
  internally via `OptionSchema::scope` / `slic3r-ffi: expose option
  scope` commit; needs the Tauri-facing summary to carry it.
- **Scope** (project / object / region bitmask) — for FR-3D-3 /
  FR-UI-9's Object-tab read-only badge on project-scope settings.
  Already exposed by the FFI per `58e199e slic3r-ffi: expose
  option scope`; needs to ride out on `OptionSummary`.
- **Capability predicate** — for FR-UI-7's printer-aware visibility.
  Each option that's only meaningful for certain printer
  capabilities (`has_toolchanger`, `has_purge_tower`, etc.) gets
  a typed predicate the frontend evaluates against the active
  printer profile. PR-4-5 consumes; PR-4-1 ships the data.

Owns the **data prerequisites** PR-4-2..PR-4-12 all consume.

**Acceptance criteria.**

- Extend `core/cascade/mod.rs::OptionSummary` with three new fields:
  ```rust
  pub struct OptionSummary {
      pub key: String,
      pub ty: String,
      pub label: Option<String>,
      pub category: Option<String>,
      pub default_value: Option<String>,
      // New:
      pub mode: OptMode,                  // Simple / Advanced / Expert / Develop
      pub scope: OptScopeFlags,           // bitmask: project=1, object=2, region=4
      pub capability: Option<CapabilityPredicate>,
  }
  ```
  serialized as a flat tagged enum on the wire so the TS side gets
  a stable shape.

- `pub enum OptMode` mirroring `ConfigOptionMode` from libslic3r —
  the existing `core/schema::OptType` pattern is the template.

- `pub struct OptScopeFlags(pub u8)` with named bit accessors
  (`is_project()`, `is_object()`, `is_region()`) — the FFI's
  bitmask layout is already documented; this is a thin Rust wrapper
  for ergonomics.

- `pub enum CapabilityPredicate` — a typed enumeration of the
  capability tests Phase 4 needs (see Open Questions in
  `phase-4.md`). Initial set, derived from the audit of
  `ConfigOptionDef::condition` predicates in
  `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp`:
  ```rust
  pub enum CapabilityPredicate {
      RequiresToolchanger,        // hide on single-extruder filament-swap printers
      RequiresPurgeTower,         // hide on toolchangers (no purging happens)
      RequiresMultiSlot,          // hide on slot_count == 1
      RequiresChamberHeater,
      // ... extend as the audit surfaces more
  }
  ```

- New Tauri command: `slicer_options_for_printer(printer: PrinterProfile)
  -> Vec<OptionSummary>` — same shape as `slicer_options` but with
  the `capability` field's hide/show pre-evaluated for the given
  printer. Frontend calls this once per printer-switch to avoid
  re-evaluating per-row.

- Tests (`core/cascade/tests` or a new perf gate alongside
  `core/cascade_perf`):
  - Mode is surfaced for representative options
    (`layer_height` = Simple, `extruder_temperature_offset` =
    Advanced/Expert).
  - Scope bitmask round-trips for project-scope (`bed_temp`),
    object-scope (`support_filament`), and region-scope
    (`wall_filament`) options.
  - Capability predicate hides `purge_volumes_matrix` on A1 mini
    (no toolchanger) and shows it on a synthetic 2-extruder
    printer profile.
  - `slicer_options_for_printer` returns ≥ 600 options for A1 mini
    in < 50 ms (10× the FR-UI panel-rerender budget so a single
    invocation never dominates).

**Effort.** ~2 days. The Mode + Scope round-trips are mechanical
since the FFI already exposes them. The CapabilityPredicate audit
is the bulk of the time — walking the OrcaSlicer source for the
`condition` lambda call sites and bucketing them.

**Dependencies.** None within Phase 4 (this is the bottom of the
graph). Builds on `58e199e slic3r-ffi: expose option scope`.

**Out of scope.** Authoring-side schema editing (cascade authors
write TOML, not a GUI). Per-setting `condition`-expression
evaluation (e.g. "show field X iff field Y == 'foo'") — that's
inter-field gating, which Phase 4 punts on per PRD §5
("simple > clever"). Capability predicates only express
printer-vs-option visibility.

**Cut candidate.** None — this is enabling infrastructure for the
rest of the phase. If `CapabilityPredicate` audit balloons, ship
the initial 5 variants and add the rest in Phase 9 polish; the
exit smoke gates on A1 mini + U1 specifically, so those two
printers' capability sets must be complete.

**Design reference.** The mockup at `docs/design/data.jsx`
declares the cascade-layer vocabulary the production
`OptionSummary` extensions should align with. Layer `id` strings
(`default`, `printer`, `build_plate`, `filament`, `user`,
`project`, `object`) and their `short` / `hue` / `desc` are the
canonical names. Use the same strings on the wire so the
frontend can route mockup CSS verbatim. The mockup's `CATEGORIES`
array is a small hand-picked subset of libslic3r's category
universe — the production `slicer_options` already returns the
full set; PR-4-3 mirrors libslic3r's category order, not the
mockup's curated short list.
