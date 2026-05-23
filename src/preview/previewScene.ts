// Three.js scene for the G-code preview (PR-6-8).
//
// Parallel to the viewport's scene mirror — owns its own
// renderer / camera / controls / scene tree. Mounted by the
// `<GcodePreview/>` React component; the scene tracks the
// loaded preview handle and rebuilds geometry on prop changes.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

import { disposeBedGrid, makeBedGrid } from "./bedGrid";
import {
  buildPreviewBuffers,
  swapExtrusionColors,
  type PreviewBuffers,
} from "./geometryBuilder";
import {
  makeExtrusionMaterial,
  setLayerWindow,
  type ExtrusionMaterial,
} from "./shaderMaterial";
import type { BoundingBox, LayerWindow, PreviewLoadResponse } from "./types";

const TRAVEL_COLOR = 0x808080;
const RETRACTION_COLOR = 0xff4444;
const RETRACTION_DOT_SIZE = 1.5;

/** Internal state the React component owns. Created in
 * `mountPreviewScene`, returned to the caller which holds a ref
 * + updates it via the `update*` methods. */
export interface PreviewScene {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  controls: OrbitControls;
  extrusionMesh: THREE.LineSegments | null;
  extrusionMaterial: ExtrusionMaterial;
  travelMesh: THREE.LineSegments | null;
  travelMaterial: THREE.LineBasicMaterial;
  retractionMesh: THREE.Points | null;
  retractionMaterial: THREE.PointsMaterial;
  bedGroup: THREE.Group | null;
  raycaster: THREE.Raycaster;
  rafHandle: number | null;
  dispose: () => void;
}

/** Mount a fresh preview scene into `container`. Renders
 * continuously via `requestAnimationFrame` for smooth orbit
 * damping; the RAF loop terminates on `dispose`. */
export function mountPreviewScene(container: HTMLElement): PreviewScene {
  const width = Math.max(container.clientWidth, 1);
  const height = Math.max(container.clientHeight, 1);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(width, height);
  // Slightly lighter than the viewport's background so the
  // mode toggle's visual context is unambiguous.
  renderer.setClearColor(0x0f1115);
  container.appendChild(renderer.domElement);

  const scene = new THREE.Scene();

  const camera = new THREE.PerspectiveCamera(45, width / height, 0.5, 5000);
  camera.up.set(0, 0, 1);
  camera.position.set(200, -200, 200);
  camera.lookAt(0, 0, 0);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.target.set(0, 0, 0);

  const extrusionMaterial = makeExtrusionMaterial();
  const travelMaterial = new THREE.LineBasicMaterial({
    color: TRAVEL_COLOR,
    transparent: true,
    opacity: 0.4,
  });
  const retractionMaterial = new THREE.PointsMaterial({
    color: RETRACTION_COLOR,
    size: RETRACTION_DOT_SIZE,
    sizeAttenuation: true,
  });

  let rafHandle: number | null = null;
  const animate = (): void => {
    controls.update();
    renderer.render(scene, camera);
    rafHandle = requestAnimationFrame(animate);
  };
  rafHandle = requestAnimationFrame(animate);

  const state: PreviewScene = {
    renderer,
    scene,
    camera,
    controls,
    extrusionMesh: null,
    extrusionMaterial,
    travelMesh: null,
    travelMaterial,
    retractionMesh: null,
    retractionMaterial,
    bedGroup: null,
    raycaster: new THREE.Raycaster(),
    rafHandle,
    dispose: () => disposeScene(state, container),
  };

  return state;
}

/** Load a fresh preview into the scene. Replaces any existing
 * extrusion / travel / retraction meshes. */
export function setPreviewBuffers(
  scene: PreviewScene,
  bytes: ArrayBuffer,
  response: PreviewLoadResponse,
): void {
  // Free the previous geometries.
  clearMesh(scene, "extrusion");
  clearMesh(scene, "travel");
  clearMesh(scene, "retraction");

  const buffers = buildPreviewBuffers(bytes, response);
  attachBuffers(scene, buffers);

  // Default layer window: full range.
  setLayerWindow(
    scene.extrusionMaterial,
    0,
    Math.max(0, response.layer_count - 1),
  );

  // Initial camera: look down +Z from above-and-behind the bbox.
  frameOnBoundingBox(scene, response.bounding_box);
}

/** Replace only the color attribute on the existing extrusion
 * geometry — used when the user swaps color modes. Cheaper than
 * `setPreviewBuffers` because the position + layer buffers
 * survive. */
export function setPreviewColors(
  scene: PreviewScene,
  bytes: ArrayBuffer,
  response: PreviewLoadResponse,
): void {
  if (!scene.extrusionMesh) return;
  swapExtrusionColors(scene.extrusionMesh.geometry, bytes, response);
}

/** Update the GPU layer-cull uniforms. PR-6-9's slider calls this
 * on every scrub tick. */
export function applyLayerWindow(
  scene: PreviewScene,
  window: LayerWindow,
): void {
  const { min, max } = layerWindowBounds(window);
  setLayerWindow(scene.extrusionMaterial, min, max);
}

/** Toggle travel/retraction visibility. PR-6-10 calls this. */
export function setVisibility(
  scene: PreviewScene,
  showTravels: boolean,
  showRetractions: boolean,
): void {
  if (scene.travelMesh) scene.travelMesh.visible = showTravels;
  if (scene.retractionMesh) scene.retractionMesh.visible = showRetractions;
}

/** Render or replace the bed grid. Caller supplies extents from
 * the active plate; passing `null` removes any existing bed. */
export function setBed(
  scene: PreviewScene,
  extents: BoundingBox | null,
): void {
  if (scene.bedGroup) {
    scene.scene.remove(scene.bedGroup);
    disposeBedGrid(scene.bedGroup);
    scene.bedGroup = null;
  }
  if (extents) {
    scene.bedGroup = makeBedGrid(extents);
    scene.scene.add(scene.bedGroup);
  }
}

/** Raycast against the extrusion `LineSegments` and return the
 * matched segment index (the Tauri-side index PR-6-7's
 * `preview_segment_detail` expects), or `null` on a miss.
 *
 * `(ndcX, ndcY)` are normalized device coordinates — `[-1, 1]`
 * with Y flipped from screen-space. Caller is responsible for
 * the conversion (PR-6-11's GcodePreview does it from
 * pointer events).
 *
 * Three.js's raycaster returns the vertex index it hit; for
 * `LineSegments` the index is per-vertex (2 per segment), so we
 * divide by 2 to recover the segment id.
 */
export function pickSegment(
  scene: PreviewScene,
  ndcX: number,
  ndcY: number,
): number | null {
  if (!scene.extrusionMesh) return null;
  scene.raycaster.setFromCamera(
    { x: ndcX, y: ndcY } as THREE.Vector2,
    scene.camera,
  );
  // Tune the raycaster's line threshold so thin segments are
  // pickable. Default is 1; bump to ~1.5mm at the current bbox
  // scale (matches the rendered line thickness perceptually).
  const params = scene.raycaster.params.Line;
  if (params) params.threshold = 1.5;
  const hits = scene.raycaster.intersectObject(
    scene.extrusionMesh,
    false,
  );
  if (hits.length === 0) return null;
  const hit = hits[0];
  // index here is the per-vertex index into the position
  // attribute; segment id = floor(index / 2).
  if (hit.index == null) return null;
  return Math.floor(hit.index / 2);
}

/** Resize the renderer + camera to the container's new size.
 * Call from a ResizeObserver or window resize handler. */
export function resizePreview(
  scene: PreviewScene,
  width: number,
  height: number,
): void {
  scene.renderer.setSize(Math.max(width, 1), Math.max(height, 1));
  scene.camera.aspect = width / Math.max(height, 1);
  scene.camera.updateProjectionMatrix();
}

// ───────────────────── private helpers ───────────────────────

function attachBuffers(scene: PreviewScene, buffers: PreviewBuffers): void {
  if (buffers.extrusionCount > 0) {
    const mesh = new THREE.LineSegments(
      buffers.extrusionGeometry,
      scene.extrusionMaterial,
    );
    mesh.name = "n3o:preview-extrusions";
    scene.extrusionMesh = mesh;
    scene.scene.add(mesh);
  } else {
    buffers.extrusionGeometry.dispose();
  }

  if (buffers.travelCount > 0) {
    const mesh = new THREE.LineSegments(
      buffers.travelGeometry,
      scene.travelMaterial,
    );
    mesh.name = "n3o:preview-travels";
    // Travels default to hidden — PR-6-10 toggles them on.
    mesh.visible = false;
    scene.travelMesh = mesh;
    scene.scene.add(mesh);
  } else {
    buffers.travelGeometry.dispose();
  }

  if (buffers.retractionCount > 0) {
    const mesh = new THREE.Points(
      buffers.retractionGeometry,
      scene.retractionMaterial,
    );
    mesh.name = "n3o:preview-retractions";
    mesh.visible = false;
    scene.retractionMesh = mesh;
    scene.scene.add(mesh);
  } else {
    buffers.retractionGeometry.dispose();
  }
}

function clearMesh(
  scene: PreviewScene,
  which: "extrusion" | "travel" | "retraction",
): void {
  const slot =
    which === "extrusion"
      ? "extrusionMesh"
      : which === "travel"
        ? "travelMesh"
        : "retractionMesh";
  const mesh = scene[slot];
  if (mesh) {
    scene.scene.remove(mesh);
    mesh.geometry.dispose();
    // Material is shared (scene.*Material); don't dispose here.
    (scene as unknown as Record<string, unknown>)[slot] = null;
  }
}

function frameOnBoundingBox(
  scene: PreviewScene,
  bbox: BoundingBox,
): void {
  const cx = (bbox.min[0] + bbox.max[0]) * 0.5;
  const cy = (bbox.min[1] + bbox.max[1]) * 0.5;
  const cz = (bbox.min[2] + bbox.max[2]) * 0.5;
  const size = Math.max(
    bbox.max[0] - bbox.min[0],
    bbox.max[1] - bbox.min[1],
    bbox.max[2] - bbox.min[2],
    20.0,
  );
  scene.controls.target.set(cx, cy, cz);
  scene.camera.position.set(cx + size * 1.5, cy - size * 1.5, cz + size * 1.5);
  scene.controls.update();
}

/** Convert a [`LayerWindow`] into the (min, max) uniform pair
 * the shader's cull condition uses. Exported for unit tests. */
export function layerWindowBounds(window: LayerWindow): {
  min: number;
  max: number;
} {
  switch (window.mode) {
    case "single":
      return { min: window.layer, max: window.layer };
    case "up-to":
      return { min: 0, max: window.max };
    case "range":
      return { min: window.min, max: window.max };
  }
}

function disposeScene(scene: PreviewScene, container: HTMLElement): void {
  if (scene.rafHandle != null) {
    cancelAnimationFrame(scene.rafHandle);
    scene.rafHandle = null;
  }
  clearMesh(scene, "extrusion");
  clearMesh(scene, "travel");
  clearMesh(scene, "retraction");
  if (scene.bedGroup) {
    scene.scene.remove(scene.bedGroup);
    disposeBedGrid(scene.bedGroup);
    scene.bedGroup = null;
  }
  scene.extrusionMaterial.dispose();
  scene.travelMaterial.dispose();
  scene.retractionMaterial.dispose();
  scene.controls.dispose();
  scene.renderer.dispose();
  if (scene.renderer.domElement.parentElement === container) {
    container.removeChild(scene.renderer.domElement);
  }
}
