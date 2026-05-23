// Parse the binary preview buffer into Three.js BufferGeometry +
// auxiliary point data (PR-6-8).
//
// Binary layout (little-endian f32; counts come from the
// PreviewLoadResponse):
//
//   [extrusion positions: 6 × extrusion_count]
//   [extrusion colors:    6 × extrusion_count]
//   [extrusion layers:    2 × extrusion_count]
//   [travel positions:    6 × travel_count]
//   [travel layers:       2 × travel_count]
//   [retraction positions: 3 × retraction_count]
//   [retraction layers:    1 × retraction_count]

import * as THREE from "three";

import type { PreviewLoadResponse } from "./types";

export interface PreviewBuffers {
  extrusionGeometry: THREE.BufferGeometry;
  travelGeometry: THREE.BufferGeometry;
  retractionGeometry: THREE.BufferGeometry;
  /** Counts surfaced for downstream consumers (slider, stats). */
  extrusionCount: number;
  travelCount: number;
  retractionCount: number;
}

/** Decode `bytes` against the counts from `response` into Three.js
 * geometry. Each geometry has the `position` attribute set;
 * extrusions additionally have `color` (per-vertex RGB) and
 * `aLayer` (per-vertex layer index, shader cull driver). */
export function buildPreviewBuffers(
  bytes: ArrayBuffer,
  response: PreviewLoadResponse,
): PreviewBuffers {
  const view = new Float32Array(bytes);
  let offset = 0;

  const ext = response.extrusion_count;
  const tra = response.travel_count;
  const ret = response.retraction_count;

  const extPos = view.subarray(offset, offset + ext * 6);
  offset += ext * 6;
  const extColor = view.subarray(offset, offset + ext * 6);
  offset += ext * 6;
  const extLayer = view.subarray(offset, offset + ext * 2);
  offset += ext * 2;
  const traPos = view.subarray(offset, offset + tra * 6);
  offset += tra * 6;
  const traLayer = view.subarray(offset, offset + tra * 2);
  offset += tra * 2;
  const retPos = view.subarray(offset, offset + ret * 3);
  offset += ret * 3;
  const retLayer = view.subarray(offset, offset + ret * 1);
  offset += ret * 1;

  if (offset !== view.length) {
    console.warn(
      `[preview] buffer offset mismatch: consumed ${offset}, payload ${view.length}`,
    );
  }

  const extrusionGeometry = new THREE.BufferGeometry();
  // Float32BufferAttribute requires copies — `subarray` is a
  // view, but Three.js mutates the array internally on dispose,
  // so we slice to detach the parent ArrayBuffer.
  extrusionGeometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(extPos.slice(), 3),
  );
  extrusionGeometry.setAttribute(
    "color",
    new THREE.Float32BufferAttribute(extColor.slice(), 3),
  );
  extrusionGeometry.setAttribute(
    "aLayer",
    new THREE.Float32BufferAttribute(extLayer.slice(), 1),
  );

  const travelGeometry = new THREE.BufferGeometry();
  travelGeometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(traPos.slice(), 3),
  );
  travelGeometry.setAttribute(
    "aLayer",
    new THREE.Float32BufferAttribute(traLayer.slice(), 1),
  );

  const retractionGeometry = new THREE.BufferGeometry();
  retractionGeometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(retPos.slice(), 3),
  );
  retractionGeometry.setAttribute(
    "aLayer",
    new THREE.Float32BufferAttribute(retLayer.slice(), 1),
  );

  return {
    extrusionGeometry,
    travelGeometry,
    retractionGeometry,
    extrusionCount: ext,
    travelCount: tra,
    retractionCount: ret,
  };
}

/** Replace only the `color` attribute on an existing extrusion
 * geometry. Used when the user swaps color modes — saves the
 * BufferGeometry tear-down/rebuild cost. */
export function swapExtrusionColors(
  geometry: THREE.BufferGeometry,
  bytes: ArrayBuffer,
  response: PreviewLoadResponse,
): void {
  const view = new Float32Array(bytes);
  const ext = response.extrusion_count;
  // Skip positions (first section); colors start at offset 6 * ext.
  const extColor = view.subarray(ext * 6, ext * 6 + ext * 6);
  geometry.setAttribute(
    "color",
    new THREE.Float32BufferAttribute(extColor.slice(), 3),
  );
  const attr = geometry.attributes.color as THREE.BufferAttribute;
  attr.needsUpdate = true;
}
