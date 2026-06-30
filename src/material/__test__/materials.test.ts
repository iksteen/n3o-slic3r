// boundMaterials.
//
// The slot-binding panel + the objects material picker must list every material
// the plate USES, not just the ones an object's `extruder_id` points at. A
// filament applied only via MMU face-painting is bound in `material_to_slot`
// with no object carrying its index — an object-derived list misses it, which is
// why an imported painted model showed "1 material" when it has 2.

import { describe, expect, it } from "vitest";
import { boundMaterials, isRfidDetected } from "../materials";
import type { PlateSnapshot, SceneObject } from "../../viewport/types";

function obj(id: number, extruder_id: number | null): SceneObject {
  return {
    id,
    mesh: id,
    transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    name: `obj-${id}`,
    visible: true,
    extruder_id,
    group: null,
  };
}

function plate(
  objects: SceneObject[],
  material_to_slot: Record<number, { extruder: number; slot: number }>,
): PlateSnapshot {
  return {
    plate_id: 1,
    name: "Plate 1",
    metadata: { composition_order: 1 },
    printer_identity: null,
    printer_instance_id: null,
    material_to_slot,
    project_overrides: {},
    quality_profile: null,
    objects,
    selection: [],
    build_plate: null,
    exclusion_zones: [],
    bed: null,
    object_overrides: {},
    groups: {},
  };
}

describe("boundMaterials", () => {
  it("includes a painted material bound only in material_to_slot", () => {
    // The base object is material 1; material 2 is applied per-face (paint),
    // so no object carries extruder_id 2 — it lives only in material_to_slot.
    const p = plate([obj(1, 1)], { 1: { extruder: 0, slot: 0 }, 2: { extruder: 0, slot: 1 } });
    expect(boundMaterials(p)).toEqual([1, 2]); // union surfaces the paint-only material 2
  });

  it("unions, dedups, and sorts object + bound materials", () => {
    const p = plate([obj(1, 3), obj(2, 1), obj(3, 1)], { 2: { extruder: 0, slot: 0 } });
    expect(boundMaterials(p)).toEqual([1, 2, 3]);
  });

  it("treats an unassigned object as material 1", () => {
    expect(boundMaterials(plate([obj(1, null)], {}))).toEqual([1]);
  });

  it("returns [] for a null plate", () => {
    expect(boundMaterials(null)).toEqual([]);
  });
});

describe("isRfidDetected", () => {
  it("detects a genuine RFID tag", () => {
    expect(isRfidDetected("F1E2D3C4B5A60718")).toBe(true);
  });

  it("treats the all-zeros sentinel as no tag", () => {
    expect(isRfidDetected("0000000000000000")).toBe(false);
    expect(isRfidDetected("0")).toBe(false);
  });

  it("treats null / empty / whitespace as no tag", () => {
    expect(isRfidDetected(null)).toBe(false);
    expect(isRfidDetected("")).toBe(false);
    expect(isRfidDetected("   ")).toBe(false);
  });
});
