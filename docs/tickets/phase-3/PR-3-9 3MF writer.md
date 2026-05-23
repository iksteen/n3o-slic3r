# PR-3-9 — Promote `core/threemf` + project-format writer (FR-MP-4)

Status: ❌ open.

**Scope.** Two things:

1. **Relocate** the project-3MF reader PR-2-4 shipped under
   `core/scene/loaders/threemf/` to its PRD §8.2-mandated home at
   `core/threemf/`. Phase 5 (project save/load) and Phase 7a
   (Bambu driver) both take a stable dep on this module; keeping
   it pinned under `scene/loaders/` couples it to scene-state
   shape changes.

2. **Write** `.3mf` project files: produce a valid 3MF Core spec
   container with `<resources>` + `<build>` + an optional `<components>`
   tree, our project-namespace metadata extensions, and BBS-flavor
   `model_settings.config` for compatibility with foreign slicers.

Owns FR-MP-4 (3MF project format).

**Acceptance criteria.**

- Move `src-tauri/src/core/scene/loaders/threemf/` →
  `src-tauri/src/core/threemf/`. Re-export the reader from
  `core/threemf/reader.rs` (or keep the existing `container.rs` /
  `core_spec.rs` / `bbs_meta.rs` split — the layout works). Update
  the dispatcher in `core/scene/loaders/mod.rs` to call into
  `crate::core::threemf::load_3mf` so PR-2-4's reader call sites
  keep working.

- `core/threemf/writer.rs`:
  - `pub struct Project3mfWriter { ... }` builder API.
  - `pub fn write(scene: &SceneState, output: &Path) ->
    Result<(), WriteError>` — given the live scene, emit a 3MF
    project file with:
    - One outer `<object>` per scene object (matching the
      object-graph shape PR-2-4 reads).
    - One inline mesh per object's `Mesh` (no cross-file
      `<components>` for now — keeps the writer simple; future
      refactor can hoist shared meshes to side files).
    - `Metadata/model_settings.config` with per-object name +
      `extruder` value pulled from `SceneObject.extruder_id`.
    - Our namespace extensions in a separate metadata file
      `Metadata/n3o_project_settings.config` (XML, schema
      defined inline in `docs/3mf-format-notes.md`) carrying:
      - active cascade override hashes (Phase 4 fills in;
        Phase 3 emits an empty placeholder so the schema is
        stable).
      - plate-printer bindings (Phase 5 fills in; Phase 3 emits
        `{}`).
      - app version + timestamp.

- Round-trip equality: for every `Project3mf` we can load via
  PR-2-4's reader, writing it back through this ticket's writer
  and reloading must produce a structurally equivalent `Project3mf`.
  Structural equivalence means:
  - same mesh count + vertex/index data (byte-equal float arrays
    after normalizing for the floating-point write precision —
    document the precision floor).
  - same object count, same per-object name + transform (within
    epsilon) + extruder id.
  - same printer hint + file metadata.

- Tests:
  - Round-trip `examples/spike3/fourcolor.3mf` through the
    writer and back through PR-2-4's reader. Assert the
    structural equivalence checks.
  - Round-trip `external/OrcaSlicer/resources/handy_models/
    OrcaCube_v2.3mf` likewise.
  - **Foreign-slicer compat:** the file we write must open
    cleanly in OrcaSlicer or Bambu Studio. Document in the
    ticket PR description how this was verified (manual open
    test, since CI doesn't have BBS installed).

- Document the writer's schema choices in
  `docs/3mf-format-notes.md`'s existing structure: which standard
  3MF elements we emit, which namespace we use for extensions,
  which BBS metadata we replicate vs. omit, and why.

**Effort.** ~3 days. Container layer + XML emission ~1.5 days,
round-trip test harness + BBS schema parity ~1 day, doc update
~0.5 day.

**Dependencies.** PR-2-4 (reader; this ticket relocates + reuses
it). Phase 5 and Phase 7a depend on this writer landing; the order
shouldn't shift them but the writer's schema choices should be
discussed inline with the Phase 5 ticket author before locking in.

**Out of scope.** PrusaSlicer-flavor write (we read it as
`Slic3r_PE_model.config` per PR-2-4 but don't author it; users
re-export through Orca for Phase 7-side compat).
`.gcode.3mf` writer — that's PR-3-10, sharing the container layer
from this ticket. Thumbnails / preview images embedded in the
project — defer to Phase 6.

**Cut candidate noted in execution plan.** Complex Bambu Studio
metadata extensions (PR-3-10's concern). This ticket's project-
format writer is *not* the cut candidate — it's a hard MVP
requirement.
