// PR-6-8 geometry-builder contract tests.
//
// Pins the binary layout the Rust side (PR-6-7's pack_buffers)
// and the TS side agree on. If either drifts, this test fails
// at the section-boundary assertion before any GPU-side
// confusion.

import { describe, expect, it } from "vitest";

import { buildPreviewBuffers, swapExtrusionColors } from "../geometryBuilder";
import type { PreviewLoadResponse } from "../types";

function fixtureResponse(
  extrusionCount: number,
  travelCount: number,
  retractionCount: number,
): PreviewLoadResponse {
  return {
    handle: 1,
    header: {
      slicer: null,
      slicer_version: null,
      estimated_time: null,
      filament_used: [],
      layer_count: null,
      object_count: null,
      printer_model: null,
      bbox_min: null,
      bbox_max: null,
      raw_settings: {},
    },
    layer_count: 2,
    extrusion_count: extrusionCount,
    travel_count: travelCount,
    retraction_count: retractionCount,
    bounding_box: { min: [0, 0, 0], max: [10, 10, 10] },
    job_stats: {
      total_duration_seconds: 0,
      layer_count: 2,
      feature_breakdown: {},
      filament_used_mm: {},
      bounding_box: { min: [0, 0, 0], max: [10, 10, 10] },
      layer_heights: { min: 0.2, max: 0.2, variable: false },
    },
  };
}

/** Pack a synthetic binary buffer matching pack_buffers' layout
 * with deterministic content so we can assert what the parser
 * extracts. */
function packFixture(
  extPositions: number[],
  extColors: number[],
  extLayers: number[],
  traPositions: number[],
  traLayers: number[],
  retPositions: number[],
  retLayers: number[],
): ArrayBuffer {
  const all = [
    ...extPositions,
    ...extColors,
    ...extLayers,
    ...traPositions,
    ...traLayers,
    ...retPositions,
    ...retLayers,
  ];
  const buf = new ArrayBuffer(all.length * 4);
  const view = new Float32Array(buf);
  view.set(all);
  return buf;
}

describe("buildPreviewBuffers", () => {
  it("decodes single-segment extrusion + travel + retraction", () => {
    const response = fixtureResponse(1, 1, 1);
    const bytes = packFixture(
      [0, 0, 0, 10, 0, 0],          // extrusion positions
      [1, 0, 0, 1, 0, 0],            // extrusion colors (red)
      [0, 0],                         // extrusion layers
      [0, 0, 0, 0, 10, 0],            // travel positions
      [0, 0],                         // travel layers
      [5, 5, 0],                      // retraction position
      [0],                            // retraction layer
    );
    const buffers = buildPreviewBuffers(bytes, response);
    expect(buffers.extrusionCount).toBe(1);
    expect(buffers.travelCount).toBe(1);
    expect(buffers.retractionCount).toBe(1);

    const extPos = buffers.extrusionGeometry.getAttribute("position");
    expect(Array.from(extPos.array)).toEqual([0, 0, 0, 10, 0, 0]);

    const extColor = buffers.extrusionGeometry.getAttribute("color");
    expect(Array.from(extColor.array)).toEqual([1, 0, 0, 1, 0, 0]);

    const extLayer = buffers.extrusionGeometry.getAttribute("aLayer");
    expect(Array.from(extLayer.array)).toEqual([0, 0]);
  });

  it("handles zero-count sections cleanly", () => {
    const response = fixtureResponse(1, 0, 0);
    const bytes = packFixture(
      [0, 0, 0, 1, 0, 0],
      [0, 1, 0, 0, 1, 0],
      [3, 3],
      [],
      [],
      [],
      [],
    );
    const buffers = buildPreviewBuffers(bytes, response);
    expect(buffers.extrusionCount).toBe(1);
    expect(buffers.travelCount).toBe(0);
    expect(buffers.retractionCount).toBe(0);
    // Zero-count geometries still have a `position` attribute
    // (empty), so the renderer can detach them without
    // null-checking everywhere.
    expect(
      buffers.travelGeometry.getAttribute("position").array.length,
    ).toBe(0);
  });

  it("swapExtrusionColors only touches the color attribute", () => {
    const response = fixtureResponse(1, 0, 0);
    const original = packFixture(
      [0, 0, 0, 1, 0, 0],
      [1, 0, 0, 1, 0, 0],
      [0, 0],
      [],
      [],
      [],
      [],
    );
    const buffers = buildPreviewBuffers(original, response);
    const positionsBefore = Array.from(
      buffers.extrusionGeometry.getAttribute("position").array,
    );

    // New buffer with positions + alternate colors.
    const swap = packFixture(
      [0, 0, 0, 1, 0, 0],
      [0, 0, 1, 0, 0, 1],
      [0, 0],
      [],
      [],
      [],
      [],
    );
    swapExtrusionColors(buffers.extrusionGeometry, swap, response);

    // Colors changed.
    const colorAfter = Array.from(
      buffers.extrusionGeometry.getAttribute("color").array,
    );
    expect(colorAfter).toEqual([0, 0, 1, 0, 0, 1]);
    // Positions unchanged.
    const positionsAfter = Array.from(
      buffers.extrusionGeometry.getAttribute("position").array,
    );
    expect(positionsAfter).toEqual(positionsBefore);
  });
});
