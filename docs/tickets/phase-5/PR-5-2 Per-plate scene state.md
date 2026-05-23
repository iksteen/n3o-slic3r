# PR-5-2 — Per-plate SceneState refactor + command migration

Status: ❌ open.

**Scope.** Today's `SceneState` (PR-2-1) is a single,
global, mutex-wrapped struct holding the entire scene —
objects, mesh buffers, active printer / build plate,
selection, gizmo mode, camera. Phase 5 needs N of these
(one per plate) without breaking the existing single-plate
worldview while the refactor lands.

This is the **critical-path bottleneck** for the rest of
Phase 5. Every other PR-5-* depends on it.

**Acceptance criteria.**

- Refactor `core/scene/state.rs::SceneState` so the
  per-plate fields move into a new `PlateSceneState`:
  ```rust
  pub struct PlateSceneState {
      pub objects: HashMap<ObjectId, SceneObject>,
      pub meshes: HashMap<MeshId, NewMesh>,
      pub selection: Vec<ObjectId>,
      pub gizmo_mode: GizmoMode,
      pub camera: Camera,
      pub bed: Option<BedMesh>,        // derived from active printer
      pub exclusion_zones: Vec<ExclusionZone>,
      pub object_overrides: HashMap<ObjectId, HashMap<String, String>>,
      // ... everything currently per-scene moves here
  }
  ```
- `SceneState` retains scene-wide registries that are
  genuinely shared across plates:
  ```rust
  pub struct SceneState {
      pub plates: Vec<PlateSceneState>,
      pub active_plate: usize,
      /// Process-wide mesh cache so identical primitives
      /// across plates share storage (PR-2-7 dedup pattern).
      pub mesh_registry: HashMap<MeshHash, NewMesh>,
      // ID allocators stay scene-wide so an object id is
      // unique across plates (move-between-plates in PR-5-11
      // needs this invariant).
      pub next_object_id: AtomicU32,
      pub next_mesh_id: AtomicU32,
  }
  ```

- **Command migration strategy:** legacy commands that don't
  take a plate id (`scene_object_translate`, `scene_select`,
  `scene_load_3mf`, etc.) **implicitly target the active
  plate.** They keep their wire shape so the Phase 2 viewport
  doesn't break. New commands that need plate-explicit
  addressing (PR-5-3's tab UI, PR-5-7's per-object overrides
  when split across plates) take a `plate_id: PlateId`
  parameter explicitly.

- **Active-plate Tauri command:** new `scene_set_active_plate(id)`
  emits a `scene:active_plate_changed { plate_id }` event so
  the frontend mirror re-syncs.

- **Snapshot path:** `scene_snapshot` returns
  `Vec<PlateSnapshot>` + `active_plate`. The frontend
  `SceneMirror` (PR-2-9) sees one event stream per plate;
  reconnect rebuilds from the snapshot.

- **Event scoping:** every `scene:*` event gains a
  `plate_id` field. `SceneEvent::ObjectAdded` becomes
  `SceneEvent::ObjectAdded { plate_id, object_id, ... }`.
  The frontend bridge filters events to the active plate's
  mirror.

- Tests:
  - Existing scene tests pass without modification (legacy
    commands implicit-targeting works).
  - New: 3-plate fixture, add cube to each plate, assert
    each plate's `objects` map is independent.
  - New: switching active plate emits the expected event +
    `scene_snapshot` returns all three plates.

**Effort.** ~5 days. The biggest ticket of Phase 5 by far.
The refactor touches every `core/scene/commands.rs` entry +
every `core/scene/state.rs` method + every `SceneEvent`
variant; mechanical but voluminous.

**Dependencies.** PR-5-1 (Project / Plate types).

**Out of scope.** UI for switching plates (PR-5-3).
Per-plate printer assignment that actually rewires the
cascade (PR-5-4). Material bindings (PR-5-6).

**Cut candidate.** None. This is the critical-path bottleneck;
shipping Phase 5 without per-plate state isn't Phase 5.

**Design reference.** The mockup's `app.jsx` `App` component
shows the per-plate state shape (each plate owns printer /
bed / nozzle / objects / overrides) — production mirrors
that exactly. Class hooks for the frontend stay the same
(`scene-mirror`, `plate-state`); the prop drilling changes
to plate-indexed.
