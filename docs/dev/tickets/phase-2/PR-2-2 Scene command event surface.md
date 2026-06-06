# PR-2-2 — Scene Tauri command + event surface

Status: ✅ done. `src-tauri/src/core/scene/{events,commands}.rs` ship the full surface: 8 SceneEvent variants (MeshLoaded / ObjectAdded / ObjectUpdated / ObjectRemoved / SelectionChanged / GizmoChanged / CameraChanged / BedChanged), 13 Tauri commands wired in `lib.rs`. Mutation methods on `SceneState` are *pure* — they return `Vec<SceneEvent>`; the Tauri layer emits each via `Window::emit` before returning. Tests bypass the Tauri runtime by exercising the pure methods directly: 10 mutation-method tests cover load→select→translate, no-op short-circuits, unknown-id handling, rotate around object center / explicit pivot, delete-clears-selection, duplicate offset, gizmo no-op, full state JSON round-trip.

**Scope.** The mutation interface the frontend uses to drive the
scene + the event interface the renderer subscribes to. Every state
change in `core/scene/` happens through this surface — the renderer
holds no authoritative state.

Mutations come in via `#[tauri::command]`s; state diffs go out via
`tauri::Window::emit()`. Each command writes the change atomically
(behind the scene-state mutex) and emits the resulting diff event
before returning. The renderer applies the diff to its local mirror;
the command returns success.

**Acceptance criteria.**

- Command surface (all `#[tauri::command]`, all `#[tracing::instrument]`):
  - `scene_load_mesh(path: String) -> MeshId` — loads via PR-2-3's
    loader, places one default object at origin.
  - `scene_select(ids: Vec<ObjectId>, mode: SelectMode)` — replace
    / add / toggle.
  - `scene_deselect()` — clear selection.
  - `scene_object_translate(id: ObjectId, delta: Vec3)`
  - `scene_object_rotate(id: ObjectId, axis: Vec3, radians: f64)`
  - `scene_object_scale(id: ObjectId, factor: Vec3)`
  - `scene_object_set_transform(id: ObjectId, transform: Transform)`
    — replaces the object's transform wholesale (used by
    Auto-arrange and the gizmo).
  - `scene_object_delete(ids: Vec<ObjectId>)`
  - `scene_object_duplicate(id: ObjectId) -> ObjectId`
  - `scene_gizmo_set(mode: GizmoMode, pivot: Option<Vec3>)`
  - `scene_camera_set(camera: CameraState)`
  - `scene_snapshot() -> SceneState` — full-state query; the
    renderer calls this on startup or after a reconnect to rebuild
    its mirror from scratch.

- Event surface (all emitted on `Window`, payload is JSON):
  - `scene:mesh_loaded` — `{ id, bounding_box, name }`
  - `scene:object_added` — `{ id, mesh_id, transform, name, ... }`
  - `scene:object_updated` — `{ id, changes: { transform?, name?, ...} }`
  - `scene:object_removed` — `{ id }`
  - `scene:selection_changed` — `{ selected: Vec<ObjectId> }`
  - `scene:gizmo_changed` — `{ mode, pivot }`
  - `scene:camera_changed` — `{ camera: CameraState }`

- The scene state lives in `tauri::State<Mutex<SceneState>>` (or
  `RwLock` if PR-2-11's perf analysis says so). Every command takes
  a `Window` parameter and emits its event before returning. Helper
  function `with_scene_mut(window, |state| -> Diff)` extracts the
  pattern.

- Unit tests cover the command/event contract *without any
  renderer attached* — pure Rust integration tests that drive
  commands directly and capture emitted events via a mock event
  sink. Sequence "load → select → translate → delete" produces the
  expected event stream.

**Effort.** ~3 days.

**Dependencies.** PR-2-1 (state types).

**Out of scope.** Renderer-side reconciliation logic — PR-2-9.
Snapshot diff *compression* (sending a minimal change set vs the
full object) — useful later but not needed for MVP; send full
updated object on `object_updated` for simplicity.

**Notes.** Mutex granularity matters for the 5 ms p99 budget. If
PR-2-11 shows lock contention, switch to a per-object lock pool or
go RwLock with copy-on-write. Don't preemptively over-engineer —
start with one Mutex<SceneState>.
