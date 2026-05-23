// Gizmo commit-pipeline tests (PR-2-10).
//
// We don't drive Three.js TransformControls end-to-end here — that
// needs a real DOM + a render pass, which is overkill for testing
// "the right command goes to Rust with the right payload." Instead
// we exercise the pure pieces of `gizmo.ts`:
//
// - `buildSetTransformPayload(id, matrix)` — the exact shape the
//   committed `invoke('scene_object_set_transform', ...)` call uses.
// - `pickAttachTargetForTest(mirror, selected)` — the object lookup
//   that decides where the gizmo attaches.
//
// Multi-select is documented as "attaches to first selected" in
// the gizmo module; tests pin that down.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  buildSetTransformPayload,
  pickAttachTargetForTest,
  transformToMatrix,
} from "../gizmo";
import { SceneMirror } from "../sceneMirror";
import type {
  CameraState,
  GizmoState,
  MeshHeader,
  PlateSnapshot,
  SceneObject,
  SceneSnapshot,
} from "../types";

function unitCubeHeader(id = 1): MeshHeader {
  return {
    id,
    vertex_count: 24,
    index_count: 36,
    bounding_box: { min: [-1, -1, -1], max: [1, 1, 1] },
    provenance: { kind: "Primitive", data: "cube" },
  };
}

function unitCubeBuffers() {
  return {
    vertices: new Float32Array(24 * 3),
    normals: new Float32Array(24 * 3),
    indices: new Uint32Array(36),
  };
}

const DEFAULT_CAMERA: CameraState = {
  position: [200, -200, 200],
  target: [0, 0, 0],
  up: [0, 0, 1],
  fov_degrees: 45,
  projection: "Perspective",
};

const DEFAULT_GIZMO: GizmoState = { mode: "None", pivot: null };

function plateSnap(id = 1): PlateSnapshot {
  return {
    plate_id: id,
    name: `Plate ${id}`,
    metadata: { cycle_count: 1, composition_order: id },
    printer: null,
    material_bindings: [],
    project_overrides: {},
    objects: [],
    selection: [],
    camera: DEFAULT_CAMERA,
    gizmo: DEFAULT_GIZMO,
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: {},
  };
}

function emptySnapshot(): SceneSnapshot {
  return {
    project_uuid: "test",
    source_path: null,
    cascade_handle: null,
    user_overrides: {},
    file_metadata: {},
    meshes: [],
    plates: [plateSnap(1)],
    active_plate_id: 1,
  };
}

async function bootMirror(): Promise<SceneMirror> {
  const mirror = new SceneMirror(async () => unitCubeBuffers());
  await mirror.applySnapshot(emptySnapshot());
  return mirror;
}

function objAt(id: number, mesh: number, tx: number): SceneObject {
  // Wire shape: Transform is `#[serde(transparent)]` over [f32; 16].
  // prettier-ignore
  const m = [
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    tx, 0, 0, 1,
  ];
  return {
    id,
    mesh,
    transform: m,
    name: `obj-${id}`,
    visible: true,
    extruder_id: null,
    parent: null,
  };
}

describe("gizmo.buildSetTransformPayload", () => {
  it("packs a Matrix4 into the column-major [16] payload Rust expects", () => {
    const m = new THREE.Matrix4();
    // Set a 10-unit X translation. In column-major, tx is index 12.
    m.makeTranslation(10, 0, 0);
    const payload = buildSetTransformPayload(42, m);
    expect(payload.id).toBe(42);
    expect(payload.transform).toHaveLength(16);
    // index 12 = tx, index 13 = ty, index 14 = tz, index 15 = 1
    expect(payload.transform[12]).toBe(10);
    expect(payload.transform[13]).toBe(0);
    expect(payload.transform[14]).toBe(0);
    expect(payload.transform[15]).toBe(1);
  });

  it("round-trips a SceneObject's matrix without loss", () => {
    const obj = objAt(7, 1, 25);
    const m = transformToMatrix(obj);
    const payload = buildSetTransformPayload(obj.id, m);
    expect(payload.transform).toEqual(obj.transform);
  });
});

describe("gizmo.pickAttachTarget", () => {
  it("returns null when nothing's selected", async () => {
    const mirror = await bootMirror();
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: objAt(101, 1, 0) },
    });
    expect(pickAttachTargetForTest(mirror, [])).toBeNull();
  });

  it("returns null when selected id doesn't exist in the mirror", async () => {
    const mirror = await bootMirror();
    expect(pickAttachTargetForTest(mirror, [999])).toBeNull();
  });

  it("finds the Three.js mesh whose userData matches the first selected id", async () => {
    const mirror = await bootMirror();
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: objAt(101, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: objAt(102, 1, 30) },
    });

    const target = pickAttachTargetForTest(mirror, [101]);
    expect(target).not.toBeNull();
    expect((target as THREE.Object3D).userData.objectId).toBe(101);
  });

  it("multi-select attaches to the FIRST selected id (PR-2-10 known limitation)", async () => {
    const mirror = await bootMirror();
    await mirror.applyEvent({
      kind: "MeshLoaded",
      data: { mesh: unitCubeHeader(1) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: objAt(201, 1, 0) },
    });
    await mirror.applyEvent({
      kind: "ObjectAdded",
      data: { plate_id: 1, object: objAt(202, 1, 30) },
    });

    const target = pickAttachTargetForTest(mirror, [201, 202]);
    expect(target).not.toBeNull();
    expect((target as THREE.Object3D).userData.objectId).toBe(201);
  });
});
