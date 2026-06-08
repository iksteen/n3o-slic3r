// Transform gizmo.
//
// Wraps three's `TransformControls`. On drag-finish the *final*
// matrix round-trips through `scene_object_set_transform` so Rust
// stays authoritative — the renderer never persists transform state
// it computed locally.
//
// Single selection attaches the gizmo directly to the mesh. Multi
// selection attaches it to a temporary pivot placed at the selection's
// bounding-box centre; the selected meshes are re-parented under the
// pivot for the duration of the drag, so translate / rotate / scale
// apply to all of them in unison around the shared centre (Three.js
// hierarchy does the maths). On drag-finish each mesh is re-parented
// back and its final matrix committed — one command per object.
//
// Snap defaults match common slicer UX: 1 mm translate snap, 15°
// rotate snap, no scale snap. Hold Shift during drag to disable
// snap (Three.js native).

import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { TransformControls } from "three/examples/jsm/controls/TransformControls.js";
import type { GizmoMode, ObjectId, SceneObject } from "./types";
import type { SceneMirror } from "./sceneMirror";

const TRANSLATE_SNAP_MM = 1.0;
const ROTATE_SNAP_DEG = 15;

/** Public surface the viewport uses to drive the gizmo. */
export interface GizmoApi {
  setMode(mode: GizmoMode): void;
  setSelection(ids: ObjectId[]): void;
  /** Re-sync the gizmo to the current selection's live transforms after a
   *  programmatic change (e.g. auto-orient), so the multi-selection pivot
   *  re-centers on the moved objects. No-op while a drag is in progress. */
  resync(): void;
  /** Hide the gizmo handles and disable interaction (e.g. while a
   *  lay-flat face pick owns the canvas), or restore them. Survives
   *  `refresh()` — a refresh while suppressed keeps the handles hidden. */
  setSuppressed(value: boolean): void;
  dispose(): void;
  /** Underlying TransformControls. The viewport adds it as a helper
   *  to the scene + listens to dragging-changed for OrbitControls
   *  pause. */
  controls: TransformControls;
}

export interface GizmoDeps {
  camera: THREE.Camera;
  domElement: HTMLElement;
  scene: THREE.Scene;
  mirror: SceneMirror;
  /** Tauri invoke hook — injectable so tests can swap in a stub. */
  invoke?: typeof invoke;
}

export function createGizmo(deps: GizmoDeps): GizmoApi {
  const invokeFn = deps.invoke ?? invoke;
  const controls = new TransformControls(deps.camera, deps.domElement);
  controls.setTranslationSnap(TRANSLATE_SNAP_MM);
  controls.setRotationSnap(THREE.MathUtils.degToRad(ROTATE_SNAP_DEG));
  // In three.js 0.169+ TransformControls is an EventDispatcher,
  // not an Object3D — its visual handles live on a separate helper
  // object retrieved via getHelper(). Add the helper to the scene.
  const helper = controls.getHelper();
  deps.scene.add(helper);

  let mode: GizmoMode = "Translate";
  let selected: ObjectId[] = [];
  // While true the handles are hidden and interaction is off (a face pick
  // owns the canvas). refresh() honours it so a re-sync doesn't reveal them.
  let suppressed = false;
  // Single-object drag: the mesh's matrix captured at drag start.
  let dragStartMatrix: THREE.Matrix4 | null = null;
  // Multi-object drag: a reusable pivot the selected meshes re-parent
  // under for the drag, plus those meshes (with original parents + ids)
  // so they can be re-parented back and committed on drag-finish.
  let pivot: THREE.Object3D | null = null;
  let pivotChildren: {
    mesh: THREE.Mesh;
    parent: THREE.Object3D;
    id: ObjectId;
  }[] = [];

  controls.addEventListener("dragging-changed", (ev) => {
    const dragging = (ev as { value: boolean }).value;
    if (dragging) onDragStart();
    else onDragEnd();
  });

  function selectedMeshes(): { mesh: THREE.Mesh; id: ObjectId }[] {
    const out: { mesh: THREE.Mesh; id: ObjectId }[] = [];
    for (const id of selected) {
      const mesh = deps.mirror.findActiveMesh(id);
      if (mesh) out.push({ mesh, id });
    }
    return out;
  }

  function onDragStart() {
    const attached = controls.object;
    if (!attached) return;
    if (pivot && attached === pivot) {
      // Re-parent the selected meshes under the pivot (preserving world
      // transform) so the gizmo moves/rotates/scales them as one.
      pivotChildren = [];
      for (const { mesh, id } of selectedMeshes()) {
        const parent = mesh.parent;
        if (!parent) continue;
        pivotChildren.push({ mesh, parent, id });
        pivot.attach(mesh);
      }
    } else {
      dragStartMatrix = attached.matrix.clone();
    }
  }

  function onDragEnd() {
    if (pivot && controls.object === pivot && pivotChildren.length > 0) {
      // Re-parent each mesh back (preserving world transform) and commit
      // its final matrix — one command per object; Rust re-applies via
      // the snapshot.
      for (const { mesh, parent, id } of pivotChildren) {
        parent.attach(mesh);
        mesh.updateMatrix();
        void invokeFn("scene_object_set_transform", {
          id,
          transform: matrixToArray(mesh.matrix),
        });
      }
      pivotChildren = [];
      // Re-centre the pivot (back to identity) on the moved meshes so the
      // next drag starts from a clean, axis-aligned frame instead of
      // inheriting this drag's rotation/scale.
      refresh();
    } else if (dragStartMatrix) {
      const attached = controls.object;
      if (attached && selected.length > 0) {
        void invokeFn("scene_object_set_transform", {
          id: selected[0],
          transform: matrixToArray(attached.matrix.clone()),
        });
      }
      dragStartMatrix = null;
    }
  }

  function detachPivot() {
    if (pivot?.parent) pivot.parent.remove(pivot);
  }

  function refresh() {
    const meshes = selectedMeshes();
    if (meshes.length === 0) {
      controls.detach();
      detachPivot();
      helper.visible = false;
      return;
    }
    if (meshes.length === 1) {
      detachPivot();
      controls.attach(meshes[0].mesh);
    } else {
      // Pivot at the selection's bounding-box centre, in the meshes'
      // parent (the active plate's object group).
      const group = meshes[0].mesh.parent ?? deps.scene;
      if (!pivot) {
        pivot = new THREE.Object3D();
        pivot.name = "n3o:gizmo-pivot";
      }
      if (pivot.parent !== group) group.add(pivot);
      const box = new THREE.Box3();
      for (const { mesh } of meshes) box.expandByObject(mesh);
      pivot.position.copy(box.getCenter(new THREE.Vector3()));
      pivot.quaternion.identity();
      pivot.scale.set(1, 1, 1);
      pivot.updateMatrixWorld(true);
      controls.attach(pivot);
    }
    helper.visible = !suppressed;
    controls.setMode(modeForThree(mode));
  }

  return {
    controls,
    setMode(m) {
      mode = m;
      refresh();
    },
    setSelection(ids) {
      selected = [...ids];
      refresh();
    },
    resync() {
      // Skip mid-drag: the drag owns the gizmo/pivot and re-attaching would
      // tear down the active manipulation.
      if (controls.dragging) return;
      refresh();
    },
    setSuppressed(value) {
      suppressed = value;
      controls.enabled = !value;
      refresh();
    },
    dispose() {
      controls.detach();
      detachPivot();
      // three.js 0.169's `TransformControls.dispose()` is buggy:
      // it calls `this.traverse(...)` but TransformControls no longer
      // extends Object3D in 0.169+ (it extends Controls/EventDispatcher),
      // so the traversal crashes the StrictMode dev-mode cleanup and
      // blanks the screen. Walk the helper ourselves — it's still an
      // Object3D and owns all the disposable resources. Fixed upstream
      // in three.js 0.171+; revisit when we bump.
      helper.traverse((node) => {
        const mesh = node as THREE.Mesh;
        if (mesh.geometry) mesh.geometry.dispose();
        const mat = mesh.material as
          | THREE.Material
          | THREE.Material[]
          | undefined;
        if (Array.isArray(mat)) {
          for (const m of mat) m.dispose();
        } else if (mat) {
          mat.dispose();
        }
      });
      deps.scene.remove(helper);
      controls.disconnect();
    },
  };
}

function pickAttachTarget(
  mirror: SceneMirror,
  selected: ObjectId[],
): THREE.Object3D | null {
  // Find the first selected object's Three.js mesh on the **active
  // plate**. PR-5-2 phase C routes the gizmo to the active plate
  // only — multi-plate selection is incoherent (you can't drag an
  // object that lives on a non-visible plate).
  for (const id of selected) {
    const mesh = mirror.findActiveMesh(id);
    if (mesh) return mesh;
  }
  return null;
}

function modeForThree(mode: GizmoMode): "translate" | "rotate" | "scale" {
  switch (mode) {
    case "Translate":
      return "translate";
    case "Rotate":
      return "rotate";
    case "Scale":
      return "scale";
    default:
      return "translate";
  }
}

function matrixToArray(m: THREE.Matrix4): number[] {
  // `Matrix4.elements` is column-major — same encoding the Rust side
  // expects on `Transform { matrix: [f32; 16] }`.
  return Array.from(m.elements);
}

/** Exported only for tests so they don't need to reach into the
 *  TransformControls drag plumbing. Builds the same commit payload
 *  the live `commitDrag` would. Wire shape: `transform` is a bare
 *  16-element column-major array (Rust's `Transform` is
 *  `#[serde(transparent)]` over `[f32; 16]`). */
export function buildSetTransformPayload(
  id: ObjectId,
  matrix: THREE.Matrix4,
): { id: ObjectId; transform: number[] } {
  return {
    id,
    transform: matrixToArray(matrix),
  };
}

/** Test helper: locate the same Object3D the live gizmo would attach
 * to, given a selection. */
export function pickAttachTargetForTest(
  mirror: SceneMirror,
  selected: ObjectId[],
): THREE.Object3D | null {
  return pickAttachTarget(mirror, selected);
}

/** Snapshot of the SceneObject's transform — used by tests to
 * exercise the commit path without spinning up TransformControls. */
export function transformToMatrix(obj: SceneObject): THREE.Matrix4 {
  return new THREE.Matrix4().fromArray(obj.transform as number[]);
}
