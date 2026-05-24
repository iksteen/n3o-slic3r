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

// Depth-fade range — how many layers below the top before
// extrusions go fully transparent. 25 layers ≈ 5mm at 0.2mm
// layer height: a single perimeter loop's worth of context.
// Tunable; the user-facing tradeoff is "deeper history visible"
// vs "top layer pops".
const float FADE_LAYERS = 25.0;
const float MIN_OPACITY = 0.10;

void main() {
  if (vLayer < uLayerMin - 0.5 || vLayer > uLayerMax + 0.5) {
    discard;
  }
  // Depth from the current top of the window. 0 = top, larger
  // values = older layers. Fades linearly to MIN_OPACITY so
  // up-to-N mode reads as "top layer emphasized, older layers
  // for context" rather than 200-deep spaghetti.
  float depth = uLayerMax - vLayer;
  float fade = mix(1.0, MIN_OPACITY, clamp(depth / FADE_LAYERS, 0.0, 1.0));
  gl_FragColor = vec4(vColor, fade);
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
 * the same scene.
 *
 * `transparent: true` + `depthWrite: false` lets the depth-fade
 * shader emit per-fragment alpha without z-fighting between
 * stacked layers. The cost is unsorted blending — fine here
 * since the layers are color-coded and the fade is monotone in
 * layer depth, so order errors aren't visually disruptive. */
export function makeExtrusionMaterial(): ExtrusionMaterial {
  const mat = new THREE.ShaderMaterial({
    vertexShader: VERTEX_SHADER,
    fragmentShader: FRAGMENT_SHADER,
    vertexColors: true,
    transparent: true,
    depthWrite: false,
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
