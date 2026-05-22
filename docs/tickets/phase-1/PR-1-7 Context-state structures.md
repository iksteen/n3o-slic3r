# PR-1-7 — Context-state structures

Status: ❌ open.

**Scope.** Typed Rust shapes for the three context-state objects
the resolver consumes: **printer profile** (slot_count, supported
build plates, per-slot toolhead config, exclusion zones, build
volume), **build plate** (identity, declared surface properties),
**filament profile** (identity, declared base type). These are
*context state*, not cascade rules — they're loaded once per
printer / per project and read by the resolver during predicate
evaluation.

Living in `core/printer/`, `core/scene/` (build plate is a Phase 2
concept), and `core/filament/` respectively (per PRD §8.2). This
ticket creates the shapes and serializers; population from JSON
files is each respective phase's work.

**Acceptance criteria.**

- `core/printer/profile.rs`:
  ```rust
  pub struct PrinterProfile {
      pub model: String,             // "Bambu A1 mini"
      pub slot_count: usize,         // 4 for A1 mini AMS, 4 for U1 toolchanger
      pub supported_build_plates: Vec<BuildPlateRef>,
      pub toolheads: Vec<Toolhead>,  // 1 for A1 mini (single nozzle), 4 for U1
      pub build_volume: BoundingBox,
      pub exclusion_zones: Vec<BoundingBox>,
  }
  pub struct Toolhead {
      pub nozzle_diameter: f64,
      pub hotend_type: String,
      pub max_temp: f64,
      // ... per-slot config the cascade resolver needs to know about
  }
  ```

- `core/scene/build_plate.rs`:
  ```rust
  pub struct BuildPlate {
      pub identity: String,          // "Textured PEI"
      pub libslic3r_curr_bed_type: String,  // "Textured PEI Plate"
      pub surface_kind: SurfaceKind,
  }
  ```

- `core/filament/profile.rs`:
  ```rust
  pub struct FilamentProfile {
      pub identity: String,          // "Bambu PLA Basic"
      pub base_type: String,         // "PLA"
      pub vendor: Option<String>,
      pub color: Option<String>,
  }
  ```

- `Context` (used by PR-1-3 to evaluate predicates) is derived
  from the active printer + active build plate + active filament
  per slot:
  ```rust
  pub struct Context {
      pub printer: Arc<PrinterProfile>,
      pub plate: Arc<BuildPlate>,
      pub filaments: Vec<Arc<FilamentProfile>>,  // length matches printer.slot_count
      pub active_slot: usize,
  }
  impl Context {
      pub fn predicate_value(&self, dotted_key: &str) -> Option<&str> { ... }
  }
  ```

- `Context::predicate_value("filament.type")` returns the
  active slot's filament base type; `"plate.type"` returns the
  active plate's identity; `"printer.model"` returns the printer
  model; etc. Predicate evaluation in PR-1-3 calls this.

- Tests:
  - Build an A1 mini `PrinterProfile` with 4 slots, swap PLA into
    slot 0 and PETG into slot 1, set the plate to Textured PEI,
    and `Context::predicate_value` returns the right values for
    all five dotted keys: `printer.model`, `printer.slot_count`,
    `filament.type`, `filament.name`, `plate.type`.
  - Swapping `active_slot` from 0 to 1 changes `filament.type`
    from PLA to PETG.

**Effort.** ~2 days.

**Dependencies.** None blocking; PR-1-1's schema is a soft
dependency (printer-fixed options reference schema keys).

**Out of scope.** Loading from JSON / disk format — that's PR-1-8.
Per-slot AMS state (color, remaining length) — Phase 7 filament
sync. Build-volume rendering — Phase 2.
