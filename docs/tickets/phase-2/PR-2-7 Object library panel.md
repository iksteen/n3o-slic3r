# PR-2-7 — Object library / scaffolding panel (FR-UI-10)

Status: ✅ shipped (temperature + stringing towers ship as `.drc` in Orca's resources — surfaced as `UnsupportedFormat`; tracked as a follow-up).

**Scope.** The "scaffolding panel" on the left side of the
viewport — a catalog of meshes the user can click to add to the
active plate. Three sections:

- **Primitives** — cube / cylinder / sphere / cone / torus,
  procedurally generated to user-specified dimensions.
- **Calibration** — four built-in fixtures shipped with the app:
  - **Dimension cube** (XYZ accuracy check at known size).
  - **Temperature tower** (per-layer-band temperature ladder).
  - **Stringing tower** (retraction tuning).
  - **Material flow calibration** (per-device — Bambu's flow test
    differs from Snapmaker's; OrcaSlicer ships printer-specific
    flow models under `external/OrcaSlicer/resources/calib/
    filament_flow/`). The library returns the right one based on
    the active printer.
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
  - `library_calibration(printer: &PrinterProfile) -> Vec<CalibrationDescriptor>` —
    returns paths into `external/OrcaSlicer/resources/handy_models/`
    and `external/OrcaSlicer/resources/calib/*.3mf` for the four
    calibration tests above. The `printer` argument picks the
    right material-flow variant for the active machine (Bambu's
    `Orca-LinearFlow.3mf` vs Snapmaker's TBD per-device fixture).
    Implementer documents the selected fixture paths inline in the
    descriptor for traceability.
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
  - `library_calibration(&a1_mini)` resolves all four fixtures
    (dimension cube, temp tower, stringing tower, Bambu-flavor
    material flow). Each path exists on disk + loads via the 3MF
    loader (PR-2-4) without error.

**Effort.** ~3 days. Primitives' geometry generation is the bulk;
the catalog + dedup is mechanical.

**Dependencies.** PR-2-1 (Mesh + Object types), PR-2-2 (scene
commands), PR-2-3 (loader path for imported files), PR-2-6 (plate
origin reference).

**Out of scope.** Custom primitive sets — Phase 8 (plugin system
authors them). Drag-and-drop file loading — Phase 4 UI work. User
favorites / pinning — Phase 4.
