// PR-5-6 UI — referencedMaterials derivation.
//
// The full panel renders DOM (selects, swatches), which our vitest
// setup doesn't run. The interesting non-trivial bit is the
// "what materials are referenced by this plate's objects" walk;
// that's pure and worth pinning so a future SceneObject shape
// change surfaces here.

import { describe, expect, it } from "vitest";
import { referencedMaterials } from "../MaterialBindingPanel";
import type {
  CameraState,
  GizmoState,
  PlateSnapshot,
  SceneObject,
} from "../../viewport/types";

const CAMERA: CameraState = {
  position: [200, -200, 200],
  target: [0, 0, 0],
  up: [0, 0, 1],
  fov_degrees: 45,
  projection: "Perspective",
};
const GIZMO: GizmoState = { mode: "None", pivot: null };

function obj(id: number, extruderId: number | null): SceneObject {
  return {
    id,
    mesh: 1,
    transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    name: `obj-${id}`,
    visible: true,
    extruder_id: extruderId,
    parent: null,
  };
}

function plate(objects: SceneObject[]): PlateSnapshot {
  return {
    plate_id: 1,
    name: "Plate 1",
    metadata: { composition_order: 1 },
    printer: null,
    material_bindings: [],
    project_overrides: {},
    objects,
    selection: [],
    camera: CAMERA,
    gizmo: GIZMO,
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: {},
  };
}

describe("referencedMaterials", () => {
  it("returns empty list for null plate", () => {
    expect(referencedMaterials(null)).toEqual([]);
  });

  it("returns empty list for a plate with no objects", () => {
    expect(referencedMaterials(plate([]))).toEqual([]);
  });

  it("dedupes + sorts the set of referenced material indices", () => {
    const p = plate([obj(1, 3), obj(2, 1), obj(3, 3), obj(4, 2)]);
    expect(referencedMaterials(p)).toEqual([1, 2, 3]);
  });

  it("treats `extruder_id = null` as material 1 (libslic3r default)", () => {
    // Objects without an explicit extruder fall back to extruder 1
    // at slice time; the binding panel must surface the implicit
    // requirement so the user knows to bind slot 1.
    const p = plate([obj(1, null), obj(2, null)]);
    expect(referencedMaterials(p)).toEqual([1]);
  });

  it("mixes null + explicit indices correctly", () => {
    const p = plate([obj(1, null), obj(2, 4), obj(3, null)]);
    expect(referencedMaterials(p)).toEqual([1, 4]);
  });
});
