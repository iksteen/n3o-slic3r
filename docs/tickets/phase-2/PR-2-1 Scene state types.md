# PR-2-1 — Scene state types (Rust authoritative)

Status: ❌ open.

**Scope.** Typed Rust data model for the 3D scene under `core/scene/`.
This is the foundation everything else in Phase 2 sits on. Every
mutation goes through Tauri commands (PR-2-2); the renderer is a
read-only consumer.

Lives in `core/scene/state.rs` (or similar). Phase 1 already shipped
`core/scene/build_plate.rs` for the cascade adapter's needs — this
ticket extends `core/scene/` with the full scene model.

**Acceptance criteria.**

- `pub struct SceneState` aggregates:
  - `meshes: HashMap<MeshId, Mesh>` — registry of loaded mesh data
    (vertices, indices, normals, bounding box, source file
    provenance).
  - `objects: HashMap<ObjectId, SceneObject>` — instances placed in
    the scene: which mesh, transform, parent (for hierarchical
    grouping), name, visibility, per-object metadata (extruder
    assignment, color, etc.).
  - `selection: HashSet<ObjectId>` — currently selected objects.
  - `camera: CameraState` — position, target, up vector, projection
    mode (perspective vs orthographic — orthographic is a cut
    candidate per the Execution Plan).
  - `gizmo: GizmoState` — active mode (translate / rotate / scale /
    none), pivot, snap state.
  - `plate: ActivePlate` — references a `BuildPlate` profile +
    transform; only one for MVP (Phase 5 adds multi-plate).
  - `exclusion_zones: Vec<ExclusionZone>` — printer-specific
    no-build regions, derived from the active `PrinterProfile`.

- `MeshId` and `ObjectId` are opaque `u64` newtypes — monotonic,
  never reused, never exposed to the frontend as raw numbers (the
  Tauri command/event surface uses them as string handles).

- `Transform` is a typed 4×4 matrix wrapper plus convenience
  constructors (`translation`, `rotation_around`, `scale`, `compose`).
  The renderer applies this verbatim — no transform math lives in
  the renderer.

- All types are `Serialize` + `Deserialize`-able for the Tauri
  command/event surface and for save/load via the Phase 3 3MF
  writer.

- Unit tests cover construction + identity transforms + parent/
  child hierarchy + serde round-trip.

**Effort.** ~3 days. The mesh registry and transform math are the
bulk; selection/camera/gizmo are small.

**Dependencies.** Phase 1 closed.

**Out of scope.** Mutations themselves (PR-2-2). The Three.js
renderer that consumes events (PR-2-9). Mesh loading from files
(PR-2-3 / PR-2-4). Performance optimization beyond "structurally
amenable to the 5 ms p99 budget" — actual perf work is PR-2-11.
