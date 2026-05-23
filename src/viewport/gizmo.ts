// Transform gizmo (PR-2-10).
//
// Wraps three's `TransformControls`. On drag-finish the *final*
// matrix round-trips through `scene_object_set_transform` so Rust
// stays authoritative — the renderer never persists transform state
// it computed locally.
//
// Multi-select drag is deferred: when more than one object is
// selected the gizmo attaches only to the first selected object, and
// dragging affects that one. The pure-translate multi-object case
// would be easy to add (apply the same delta vector per object) but
// rotate / scale need careful pivot handling that's better suited to
// Phase 4 UI work. Single-object covers the dominant flow.
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
  setPivotOverride(pivot: [number, number, number] | null): void;
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

  let mode: GizmoMode = "None";
  let selected: ObjectId[] = [];
  let pivotOverride: [number, number, number] | null = null;
  /** Matrix captured at drag start; used to compute the *final*
   * matrix that gets committed via Tauri. */
  let dragStartMatrix: THREE.Matrix4 | null = null;

  controls.addEventListener("dragging-changed", (ev) => {
    const dragging = (ev as { value: boolean }).value;
    if (dragging) {
      const attached = controls.object;
      if (attached) {
        dragStartMatrix = attached.matrix.clone();
      }
    } else if (dragStartMatrix) {
      commitDrag();
      dragStartMatrix = null;
    }
  });

  function commitDrag() {
    if (selected.length === 0) return;
    const attached = controls.object;
    if (!attached) return;
    // The gizmo only attaches to the first selected mesh; the
    // committed matrix is the final state of *that* mesh. For
    // multi-select we still issue commands only for the first
    // mesh (see module-level comment).
    const finalMatrix = attached.matrix.clone();
    void invokeFn("scene_object_set_transform", {
      id: selected[0],
      transform: { matrix: matrixToArray(finalMatrix) },
    });
  }

  function refresh() {
    if (mode === "None" || selected.length === 0) {
      controls.detach();
      helper.visible = false;
      return;
    }
    const target = pickAttachTarget(deps.mirror, selected, pivotOverride);
    if (!target) {
      controls.detach();
      helper.visible = false;
      return;
    }
    controls.attach(target);
    helper.visible = true;
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
    setPivotOverride(p) {
      pivotOverride = p;
      refresh();
    },
    dispose() {
      controls.detach();
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
  pivotOverride: [number, number, number] | null,
): THREE.Object3D | null {
  // Find the first selected object's Three.js mesh. The mirror's
  // objectGroup is a flat list of meshes keyed by userData.objectId.
  for (const id of selected) {
    for (const child of mirror.objectGroup.children) {
      const meshId = (child.userData as { objectId?: ObjectId }).objectId;
      if (meshId === id) {
        if (pivotOverride) {
          // Apply pivot override by translating the mesh's matrix so
          // its origin aligns with the override before attach. We
          // don't actually mutate the world matrix — TransformControls
          // grabs the object's `position` for translate gizmos, so
          // overriding the pivot is a Phase 4+ concern that needs a
          // proxy Object3D. For MVP we ignore pivotOverride and let
          // the gizmo sit at the mesh's natural origin.
        }
        return child;
      }
    }
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
 *  the live `commitDrag` would. */
export function buildSetTransformPayload(
  id: ObjectId,
  matrix: THREE.Matrix4,
): { id: ObjectId; transform: { matrix: number[] } } {
  return {
    id,
    transform: { matrix: matrixToArray(matrix) },
  };
}

/** Test helper: locate the same Object3D the live gizmo would attach
 * to, given a selection. */
export function pickAttachTargetForTest(
  mirror: SceneMirror,
  selected: ObjectId[],
): THREE.Object3D | null {
  return pickAttachTarget(mirror, selected, null);
}

/** Snapshot of the SceneObject's transform — used by tests to
 * exercise the commit path without spinning up TransformControls. */
export function transformToMatrix(obj: SceneObject): THREE.Matrix4 {
  return new THREE.Matrix4().fromArray(obj.transform.matrix);
}
