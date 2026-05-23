// Custom ShaderMaterial with GPU layer-cull uniforms (PR-6-8).
//
// Per-vertex `aLayer: float` carries the layer index a segment
// belongs to. The fragment shader discards fragments whose layer
// falls outside `[uLayerMin, uLayerMax]`. PR-6-9's slider drives
// the uniforms — a uniform update is ~free, hits the 60fps gate
// even on a 50MB gcode's 3M segments without rebuilding the
// vertex buffer.
//
// Color is supplied as a per-vertex `color` attribute (vec3,
// 0..1). PR-6-5's encoders produce this on the Rust side; the
// renderer just binds the buffer.

import * as THREE from "three";

const VERTEX_SHADER = `
attribute float aLayer;
varying vec3 vColor;
varying float vLayer;
void main() {
  vColor = color;
  vLayer = aLayer;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

const FRAGMENT_SHADER = `
uniform float uLayerMin;
uniform float uLayerMax;
varying vec3 vColor;
varying float vLayer;
void main() {
  if (vLayer < uLayerMin - 0.5 || vLayer > uLayerMax + 0.5) {
    discard;
  }
  gl_FragColor = vec4(vColor, 1.0);
}
`;

export interface ExtrusionMaterial extends THREE.ShaderMaterial {
  uniforms: {
    uLayerMin: { value: number };
    uLayerMax: { value: number };
  };
}

/** Build a fresh ShaderMaterial for the extrusion `LineSegments`.
 * Each call returns a fresh material; share by reference across
 * the same scene. */
export function makeExtrusionMaterial(): ExtrusionMaterial {
  const mat = new THREE.ShaderMaterial({
    vertexShader: VERTEX_SHADER,
    fragmentShader: FRAGMENT_SHADER,
    vertexColors: true,
    uniforms: {
      uLayerMin: { value: 0.0 },
      uLayerMax: { value: 0.0 },
    },
  });
  // ShaderMaterial doesn't infer the right TS type when uniforms
  // are typed inline; cast to the narrower interface so callers
  // can update uniforms without `as any`.
  return mat as ExtrusionMaterial;
}

/** Apply a layer window to the material. `min` / `max` clamp to
 * `[0, layerCount-1]`; single-layer mode passes the same value
 * for both. */
export function setLayerWindow(
  mat: ExtrusionMaterial,
  min: number,
  max: number,
): void {
  mat.uniforms.uLayerMin.value = min;
  mat.uniforms.uLayerMax.value = max;
}
