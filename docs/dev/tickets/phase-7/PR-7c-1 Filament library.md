# PR-7c-1 — Filament profile library + cascade integration

Status: ❌ open.

**Scope.** Author bundled filament profiles (Generic PLA / PETG
/ ABS + Bambu-flavored variants). Custom profile editor surface
that lets the user add filaments via the same cascade-TOML
mechanism PR-1-8 used. Filament profile drives temperature /
flow / cooling defaults via the cascade layering.

**Acceptance criteria.**

- New TOML files under `profiles/filaments/`:
  - `generic-pla.toml` (already exists — review + expand if
    sparse).
  - `generic-petg.toml`
  - `generic-abs.toml`
  - `bambu-pla-basic.toml`
  - `bambu-petg-hf.toml`
  - `bambu-abs.toml`

- Each filament TOML defines, as cascade overlays:
  - `nozzle_temperature_initial_layer` + `nozzle_temperature`
  - `bed_temperature_initial_layer` + `bed_temperature` (per
    plate type via `when.build_plate.identity`)
  - `slow_down_min_speed`, `fan_min_speed`,
    `close_fan_the_first_x_layers`
  - `filament_flow_ratio`, `filament_density`,
    `filament_diameter` (default 1.75)
  - `filament_settings_id` — the printer-side identifier the
    firmware uses for filament-id matching at send-time.

- Cascade composition: filament TOML overlays load after the
  printer cascade, before user/project/object overrides. Verify
  with a test: a Bambu PLA + A1 mini + Textured PEI context
  resolves `nozzle_temperature_initial_layer` to the Bambu PLA
  TOML's value (e.g., 215) and not to the cascade default.

- **`FilamentLibrary` Rust model** (`core/filament/library.rs`):
  - `pub struct FilamentLibrary { profiles: Vec<FilamentProfile> }`
  - `pub fn bundled() -> FilamentLibrary` — loads all
    `profiles/filaments/*.toml` at startup.
  - `pub fn by_identity(&self, id: &str) -> Option<&FilamentProfile>`.
  - `pub fn families() -> Vec<&str>` — distinct
    `FilamentProfile.base_type` values (`PLA`, `PETG`,
    `ABS`, ...). Drives the auto-binding heuristic in
    PR-7c-5.

- **Custom profile editor**: out of MVP scope for the
  *editor UI* — the user adds custom filaments by dropping a
  new TOML into `~/.config/n3o-slic3r/filaments/`. The
  `bundled()` loader merges from both bundled + user dirs.

- **Tauri command surface** (`core/filament/commands.rs`):
  - `filament_library_list() -> Vec<FilamentSummary>`.
  - `filament_library_reload()` — re-scan disk for newly
    added user TOMLs.

- Tests:
  - **`bundled_loads_all_profiles`** — `bundled()` returns
    ≥ 6 profiles.
  - **`profile_overlays_resolve_to_bambu_values_for_bambu_filament`**
    — context: A1 mini + Textured PEI + Bambu PLA Basic;
    assert `nozzle_temperature_initial_layer == 215`
    (whatever value the Bambu TOML carries).
  - **`families_covers_pla_petg_abs`** — assert at minimum
    these three families appear.

**Effort.** ~1.5 days. Authoring the TOMLs against
real-world reference values is the bulk.

**Dependencies.** PR-1-2 (cascade format), PR-1-7
(`FilamentProfile` type), PR-1-8 (cascade-TOML authoring
pattern).

**Out of scope.**

- Custom profile editor UI — TOML-on-disk only for MVP.
- Importing BBS/Orca filament libraries in bulk — post-MVP.
- Drying-recommendation surfaces, per-spool calibration
  history — Phase 9+.
