# PR-2-7 — Object library / scaffolding panel (FR-UI-10)

Status: ❌ open.

**Scope.** The "scaffolding panel" on the left side of the
viewport — a catalog of meshes the user can click to add to the
active plate. Three sections:

- **Primitives** — cube / cylinder / sphere / cone / torus,
  procedurally generated to user-specified dimensions.
- **Calibration** — calibration cube, temperature tower, generic
  flow test. Built-in fixtures shipped with the app.
- **Imported** — files the user loaded in this session, available
  for re-instancing without re-loading.

Drives the UX flow "click → object appears at plate origin." Each
section is a Tauri command that returns the catalog; clicking emits
a `scene_load_mesh` or `scene_object_add_from_primitive` command.

**Acceptance criteria.**

- Three Tauri commands:
  - `library_primitives() -> Vec<PrimitiveDescriptor>` — returns
    the 5 primitive types with their parameter schemas (cube needs
    `width / height / depth`; cylinder needs `radius / height /
    segments`; etc.).
  - `library_calibration() -> Vec<CalibrationDescriptor>` — returns
    paths into `external/OrcaSlicer/resources/handy_models/` and
    `external/OrcaSlicer/resources/calib/*.3mf` for the calibration
    tests we want exposed: Orca cube, temp tower, flow test.
  - `library_imported() -> Vec<ImportedDescriptor>` — returns the
    list of meshes registered this session (their `MeshId` + name +
    bounding box).

- `scene_object_add_from_primitive(kind: PrimitiveKind, params: PrimitiveParams) -> ObjectId`:
  procedurally generates the mesh, registers it via the standard
  PR-2-2 flow, places one object at plate origin. The mesh is
  cached — calling again with the same parameters returns the
  existing `MeshId` (deduplication).

- Procedural generators in `core/scene/primitives.rs`:
  - Cube → standard triangulated box.
  - Cylinder → top + bottom cap + side strip.
  - Sphere → icosphere with configurable subdivision (default 3).
  - Cone → tip + base + side strip.
  - Torus → standard double-loop tessellation.
  Each produces a `Mesh` with per-vertex normals.

- Tests:
  - `library_primitives()` returns all 5 with sane parameter
    schemas.
  - `scene_object_add_from_primitive` with a 20×20×20 cube
    produces an object with the expected bounding box.
  - Same call twice deduplicates: two `ObjectId`s, one `MeshId`.
  - `library_calibration()` resolves at least the Orca cube path
    (other entries are nice-to-have; cube is the must-have for
    Phase 2 smoke).

**Effort.** ~3 days. Primitives' geometry generation is the bulk;
the catalog + dedup is mechanical.

**Dependencies.** PR-2-1 (Mesh + Object types), PR-2-2 (scene
commands), PR-2-3 (loader path for imported files), PR-2-6 (plate
origin reference).

**Out of scope.** Custom primitive sets — Phase 8 (plugin system
authors them). Drag-and-drop file loading — Phase 4 UI work. User
favorites / pinning — Phase 4.
