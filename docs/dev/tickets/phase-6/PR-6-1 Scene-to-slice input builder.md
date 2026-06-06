# PR-6-1 — Scene-to-slice input builder

Status: ✅ shipped.

**Scope.** Pure-function adapter that turns the live `Project`'s
state for one plate into a `SliceJobInput` libslic3r can consume.
Replaces the current "user picks a mesh file" path that ignores
everything the user actually composed in the scene.

This ticket is the first half of the
[architecture invariant](../phase-6.md#architecture-invariant--slice-is-driven-by-project-state-not-by-file-paths)
that Phase 6 establishes: **the slice path reads from `Project`,
not from disk.** No user-visible file is involved in the call;
the only file that touches disk is the internal temp `.3mf`
this ticket writes for libslic3r's loader.

**Acceptance criteria.**

- New module `core/slice/input.rs`. Suggested surface:

  ```rust
  /// Build a `SliceJobInput` from `project`'s state for `plate_id`.
  ///
  /// Writes a temp `.3mf` to `std::env::temp_dir()` containing the
  /// plate's meshes + transforms + per-volume `extruder` metadata.
  /// The returned `SliceJobInput.model_path` points at this temp
  /// file; callers are responsible for deleting it after the
  /// slice job terminates (Finished / Failed / Cancelled).
  ///
  /// Resolves the per-plate cascade context: printer binding +
  /// build plate + active filament (derived from the plate's
  /// material bindings, slot 1 → first filament unless the plate
  /// overrides). User-tier overrides apply project-wide; project-
  /// and object-tier overrides come from the plate.
  pub fn build_slice_input(
      project: &Project,
      plate_id: PlateId,
  ) -> Result<(SliceJobInput, PathBuf), SliceInputError>;

  pub enum SliceInputError {
      UnknownPlate(PlateId),
      UnboundPrinter { plate_id: PlateId },
      EmptyScene { plate_id: PlateId },
      /// Cascade context can't be assembled (e.g. printer identity
      /// doesn't resolve in the registry, no filament for slot 1).
      CascadeContextUnavailable { plate_id: PlateId, message: String },
      /// 3MF temp-file write failed.
      TempWrite { path: PathBuf, source: std::io::Error },
  }
  ```

- The temp `.3mf` is geometry-only: meshes + per-object
  transforms + per-volume `extruder` metadata (drawn from
  `SceneObject.extruder_id`). The n3o-slic3r project namespace
  (material bindings, plate metadata, etc.) is **omitted** —
  libslic3r ignores it anyway and skipping it keeps the temp
  file lean. Use the existing `core::threemf` writer with a
  `geometry_only: true` flag (add the flag in this ticket if
  the writer doesn't already support it).

- `ContextJson` population:
  - `printer` ← `plate.printer.printer_identity` resolved via
    `core::printer::lookup()`, serialized via existing
    `PrinterProfile` → `ContextJson.printer` path (PR-1-7's
    convention).
  - `plate` ← `plate.printer.build_plate_identity`, resolved
    against the printer's `supported_build_plates`.
  - `filaments` ← walk `plate.material_bindings`, sort by
    `physical_slot`, look up each `filament_identity` via the
    bundled filament catalog (PR-1-8 ships Generic PLA;
    others land as the catalog grows).
  - `active_slot` ← `0` for the MVP. Multi-slot active-slot
    rotation is a Phase 7c concern.
  - `user_overrides` ← `project.user_overrides` flattened to
    `Vec<(String, String)>`.
  - `project_overrides` ← `plate.project_overrides` likewise.
  - `object_overrides` ← `plate.scene.object_overrides`
    flattened, keyed by `ObjectId.0`.

- `plate_ids: vec![plate_id.0]` — the orchestrator only walks
  one plate per job in the MVP (multi-plate batching is a
  Phase 7 ergonomic, not a Phase 6 concern).

- Tests (`src-tauri/src/core/slice/input.rs` `#[cfg(test)] mod tests`):
  - **Happy path:** a 1-plate project with one cube on the A1
    mini → returns a valid `SliceJobInput` whose `model_path`
    is a real readable file and whose `ContextJson` carries the
    A1 mini's printer profile.
  - **Multi-plate, non-active plate:** explicit `plate_id` →
    returns the input for that plate, not the active one.
  - **Per-object extruder** survives the temp 3MF: load the
    temp file back via the threemf reader and assert each
    object's `extruder_id` round-tripped.
  - **Project + object overrides** populate `ContextJson`
    correctly.
  - **Error cases:** unknown plate, unbound printer, empty
    scene, temp-write failure (use a non-writable path).
  - **Temp file cleanup** is the caller's job — the builder
    only returns the path; the test asserts the path exists
    after the call and the test itself removes it.

**Effort.** ~1.5 days. The bulk of the work is the
`ContextJson` field-by-field translation; the temp-3MF write
piggybacks on existing PR-5-8 / PR-3-9 writers.

**Dependencies.** PR-5-1 (`Project` types), PR-5-6 (material
bindings), PR-5-7 (object overrides), PR-3-9 part 2 (threemf
writer), PR-1-7 (`SlicingContext` / `ContextJson` shape),
PR-1-8 (bundled filament catalog), PR-5-4 (`printer::lookup`
registry).

**Out of scope.**

- The Tauri command surface (PR-6-2).
- The frontend Slice button rewire (PR-6-3).
- Multi-plate batch slicing — one plate per call.
- Temp file deletion — the caller (PR-6-2's command) owns
  cleanup.
- The `geometry_only: true` flag's interaction with project
  save/load (PR-5-8 always wants the full namespace; only this
  ticket's writer call uses the flag).

**Cut candidate.** None — every later Phase 6 ticket that
exercises the slice pipeline depends on this. Without it the
preview can only show third-party G-code via the drag-drop
loader (PR-6-14).
