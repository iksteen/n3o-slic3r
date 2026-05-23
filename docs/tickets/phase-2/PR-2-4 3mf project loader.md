# PR-2-4 — `.3mf` project loader

Status: ✅ shipped.

**Scope.** Load `.3mf` files as full *project* — geometry, object
positions, plate assignment, per-volume extruder, embedded settings.
Bambu Studio, OrcaSlicer, and Snapmaker Orca all save projects as
`.3mf` with their own metadata extensions; this loader is the
migration path for users coming from those tools.

PR-0.5-3 mapped the BBS `.gcode.3mf` metadata inventory; this loader
reads the *project* shape (un-sliced 3MF), not the gcode-wrapped
one. Phase 3's 3MF writer handles the gcode wrap.

**Acceptance criteria.**

- `pub fn load_3mf<P: AsRef<Path>>(path: P) -> Result<Project3mf, LoadError>`:
  - Parses the standard 3MF container: `[Content_Types].xml`,
    `_rels/.rels`, `3D/3dmodel.model`, optionally
    `3D/Objects/object_*.model` referenced via `<component>`.
  - Extracts per-object meshes via the standard 3MF mesh element.
  - Reads BambuStudio/Orca metadata extensions when present:
    `Metadata/model_settings.config` (per-part `extruder=N`,
    per-plate object placement), `Metadata/project_settings.config`
    (the printer profile + cascade-side config the file was
    authored against — we surface this as informational; the
    cascade resolver doesn't consume it).
  - Auxiliaries (`Auxiliaries/Model Pictures/*.webp`, thumbnails)
    extracted but not used yet — Phase 4 / Phase 5 surface them.

- `Project3mf` is a typed container the scene can ingest:
  ```rust
  pub struct Project3mf {
      pub meshes: Vec<Mesh>,
      pub objects: Vec<ProjectObject>,    // mesh_idx, transform, per-part extruder
      pub plate_assignments: Vec<PlateAssignment>,  // object_idx → plate
      pub printer_hint: Option<String>,   // surfaced in load UI
      pub embedded_settings: Option<String>,  // raw project_settings.config
  }
  ```

- `scene_load_3mf(path: String)` Tauri command (extends PR-2-2's
  surface) ingests a `Project3mf` into the scene state: registers
  each `Mesh`, places each `ProjectObject` at its `Transform`,
  applies per-part extruder via `SceneObject.extruder_id`, emits
  `scene:object_added` per object.

- The per-part-extruder data is the same shape we saw in PR-0.5-3's
  fourcolor Benchy (8 parts in 1 object, each with `extruder=N`).
  This is the data the future PR-1-12 investigation needs — having
  it in our scene state in Phase 2 is what makes the investigation
  cheap in Phase 3 (and lets the cascade adapter route per-volume
  extruder correctly when Phase 5 wires multi-material slicing).

- Tests:
  - Load `examples/spike3/fourcolor.3mf` — assert 8 ProjectObjects
    with extruder assignments matching the Phase 0.5 finding.
  - Load `external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf`
    — assert single mesh, single object, default extruder.
  - Malformed 3MF (truncated zip, missing 3D/3dmodel.model) →
    typed `LoadError` with the offending file path.

**Effort.** ~5 days. The standard 3MF container is well-documented;
the BambuStudio/Orca metadata extensions need careful XML reading
+ documented in `docs/3mf-format-notes.md` (new) as a knowledge
artifact.

**Dependencies.** PR-2-1 (Mesh + SceneObject types), PR-2-2 (scene
commands + events), PR-2-3 (shared mesh loader infrastructure).

**Out of scope.** Painted-region / per-volume material data —
Phase 5 / Phase 7. Phase 3 3MF *writer* — that's Phase 3 work
(FR-GP-* + .3mf I/O). Slicing the loaded project — that's Phase 3
too.

**PrusaSlicer 3MF flavor explicitly not supported in MVP.**
PrusaSlicer writes a slightly different metadata schema
(`Slic3r_PE_model.config` vs `model_settings.config`,
`Slic3r_PE_print_config.config` vs `project_settings.config`).
Since BBS and OrcaSlicer are forks of PrusaSlicer and OrcaSlicer
already supports Prusa devices, users migrating *from*
PrusaSlicer typically re-export via OrcaSlicer first. Adding
direct PrusaSlicer 3MF support is a half-day of XML reading + a
test fixture; pick it up if a real user asks. Phase 2 just
documents the gap in the loader's error message
("PrusaSlicer-flavor 3MF detected — re-save through OrcaSlicer
for full project metadata support").
