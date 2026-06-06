# PR-5-1 — Project + Plate + Binding domain types

Status: ❌ open.

**Scope.** Define the Rust types that model the multi-plate,
multi-printer project. These are the IR every other Phase 5
ticket consumes; ship them first so subsequent tickets can
build on a stable shape.

Owns the **data prerequisites** for PR-5-2 through PR-5-12.

**Acceptance criteria.**

- New module `core/project/` (extends the existing `project::`
  module that ships `SlicingContext` — keep that intact;
  multi-plate is additive). Suggested layout:
  ```
  core/project/
    mod.rs            # re-exports + module docs
    model.rs          # Project, Plate types
    binding.rs        # PrinterBinding, MaterialBinding
    metadata.rs       # PlateMetadata (cycle count, composition order)
  ```

- Core types (`serde::{Serialize, Deserialize}` everything for
  `.3mf` save/load in PR-5-8):

  ```rust
  pub struct Project {
      pub plates: Vec<Plate>,
      /// Index into `plates` — the plate the UI is currently
      /// editing. `None` only valid in a brand-new empty
      /// project; default-constructed projects start with
      /// one plate and active = 0.
      pub active_plate: usize,
      /// Cascade handle (from PR-1-9's CascadeRegistry).
      /// One cascade per project; per-plate overrides layer
      /// on top.
      pub cascade_handle: CascadeHandle,
      /// User-tier overrides (FR-CAS-3). Apply across all
      /// plates; project-tier overrides live per-plate.
      pub user_overrides: HashMap<String, String>,
      /// 3MF file-level metadata for save round-trips.
      pub file_metadata: BTreeMap<String, String>,
      /// Where this project came from / saves to. `None`
      /// for unsaved new projects.
      pub source_path: Option<PathBuf>,
  }

  pub struct Plate {
      pub id: PlateId,
      /// Display name for the tab strip. Default
      /// "Plate 1" / "Plate 2" / …; user-renamable.
      pub name: String,
      pub printer: PrinterBinding,
      /// Project-tier overrides scoped to this plate.
      pub project_overrides: HashMap<String, String>,
      pub material_bindings: Vec<MaterialBinding>,
      pub metadata: PlateMetadata,
      /// The plate's SceneState — objects, mesh refs, active
      /// build plate, etc. Refactored from the single
      /// global SceneState in PR-5-2.
      pub scene: PlateSceneState,
  }

  /// 1-based opaque plate id. Stable across the plate list
  /// — reordering doesn't change ids, only positions.
  pub struct PlateId(pub u32);

  pub struct PrinterBinding {
      /// `profiles/printers/<identity>.toml` identity.
      pub printer_identity: String,
      /// Build plate selection within the printer's
      /// supported plates.
      pub build_plate_identity: String,
  }

  pub struct MaterialBinding {
      /// Model material index (1..N) as referenced by
      /// per-volume `extruder` metadata.
      pub model_material: u8,
      /// Physical slot index on the bound printer
      /// (1-based, matching libslic3r convention).
      pub physical_slot: u8,
      /// Filament profile identity bound to this slot.
      pub filament_identity: String,
  }

  pub struct PlateMetadata {
      /// FR-MP-7: number of times the platecycler should
      /// run this plate. Default 1; range 1-999.
      pub cycle_count: u32,
      /// FR-MP-7: position in the plate composition order
      /// (1-based). PlateCycler plugin reads this to know
      /// the print sequence. Default = plate position in
      /// `Project.plates`.
      pub composition_order: u32,
  }

  pub struct PlateSceneState { /* see PR-5-2 */ }
  ```

- Constructor + invariant helpers:
  - `Project::new(cascade_handle)` → single-plate default.
  - `Project::add_plate(printer: PrinterBinding) -> PlateId`.
  - `Project::remove_plate(id)` → errors if it's the last
    plate (a project always has ≥ 1).
  - `Project::active_plate_mut()` → ergonomic mutable
    access.
  - `Plate::default_name(index)` → `"Plate N"`.

- Tests:
  - Round-trip serde JSON for every type (assert exact
    field names since `.3mf` save/load reads + writes).
  - `Project::new` produces a 1-plate project with
    active = 0.
  - `Project::remove_plate` errors on the last plate.
  - `MaterialBinding` validates `physical_slot >= 1` and
    `model_material >= 1`.

**Effort.** ~1.5 days. Mostly type-shaping work; the
SerializeJson + invariant tests are mechanical once the
shape is set.

**Dependencies.** PR-1-9's `CascadeHandle` (`u64` newtype),
PR-1-7's `SlicingContext` (kept intact, not replaced),
PR-2-1's scene types (extended in PR-5-2).

**Out of scope.** Per-plate Tauri commands (PR-5-2).
Project save/load on disk (PR-5-8). The actual UI surfaces
(PR-5-3..-6). The autosave format / location (PR-5-10).

**Cut candidate.** None — every later ticket depends on
these types being in place.
