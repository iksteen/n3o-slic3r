// Per-face vertex colours for MMU-painted meshes in the prepare viewport.
//
// A painted object is rendered with a de-indexed (non-indexed) geometry so
// each source triangle owns its three vertices and can carry a flat colour.
// `buildFaceColors` produces the `color` BufferAttribute data: triangle `t`'s
// filament state (`states[t]`) → a 24-bit colour via `colorForState`, written
// to all three of that triangle's vertices.
//
// Pure + Three-light (only THREE.Color for the hex→rgb conversion) so it unit
// tests without a renderer.

import * as THREE from "three";

/** Build the flat per-face vertex-colour buffer for a de-indexed painted
 *  mesh. Length is `states.length * 3 vertices * 3 channels`. `colorForState`
 *  maps a per-triangle filament state to a 24-bit sRGB colour (the caller
 *  resolves state → material → slot → colour). */
export function buildFaceColors(
  states: Uint8Array,
  colorForState: (state: number) => number,
): Float32Array {
  const triangleCount = states.length;
  const colors = new Float32Array(triangleCount * 9);
  const c = new THREE.Color();
  for (let t = 0; t < triangleCount; t++) {
    // THREE.Color stores linear-space rgb after a sRGB setHex (default), which
    // is exactly what a vertex-colour buffer expects — matching the non-painted
    // path's `material.color.setHex`.
    c.setHex(colorForState(states[t]));
    const base = t * 9;
    for (let k = 0; k < 3; k++) {
      const v = base + k * 3;
      colors[v] = c.r;
      colors[v + 1] = c.g;
      colors[v + 2] = c.b;
    }
  }
  return colors;
}
