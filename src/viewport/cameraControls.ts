// Camera helpers for the viewport (PR-2-9).
//
// Wraps three's `OrbitControls` with `frameBox` / `initialFrameForBed`
// helpers. The viewport uses a single perspective camera — the
// orthographic projection + its toggle were dropped.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { BedMesh } from "./types";

/** Make a camera matching `state`. Bed-side `up` is +Z (mm slicer
 * convention); we don't use Y-up like web defaults. */
export function makePerspectiveCamera(aspect: number): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, aspect, 0.5, 5000);
  cam.up.set(0, 0, 1);
  cam.position.set(200, -200, 200);
  cam.lookAt(0, 0, 0);
  return cam;
}

export function makeControls(
  camera: THREE.Camera,
  dom: HTMLElement,
): OrbitControls {
  const controls = new OrbitControls(camera, dom);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.zoomSpeed = 1.2;
  controls.target.set(0, 0, 0);
  return controls;
}

/** Adjust camera + controls so `box` fits the viewport with a 1.4×
 * margin, preserving the current view direction. Module-private —
 * only `initialFrameForBed` uses it. */
function frameBox(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  box: THREE.Box3,
): void {
  if (box.isEmpty()) return;
  const size = new THREE.Vector3();
  box.getSize(size);
  const center = new THREE.Vector3();
  box.getCenter(center);

  const maxDim = Math.max(size.x, size.y, size.z) || 1;
  const margin = 1.4;

  const fov = (camera.fov * Math.PI) / 180;
  const distance = (maxDim * 0.5 * margin) / Math.tan(fov * 0.5);
  // Preserve current view direction by reading the camera's existing
  // offset from the target and rescaling.
  const direction = camera.position.clone().sub(controls.target).normalize();
  if (direction.lengthSq() < 1e-6) {
    direction.set(1, -1, 1).normalize();
  }
  camera.position.copy(center).addScaledVector(direction, distance);
  controls.target.copy(center);
  camera.near = Math.max(distance * 0.01, 0.1);
  camera.far = distance * 100;
  camera.updateProjectionMatrix();
  controls.update();
}

/** Compute a sensible initial view given the bed's extents — pull
 * the camera back along (+X, -Y, +Z) so the user sees the front-left
 * corner. Called before the first frame so there's *something* on
 * screen even when the scene is empty. */
export function initialFrameForBed(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  bed: BedMesh,
): void {
  const box = new THREE.Box3(
    new THREE.Vector3(...bed.extents.min),
    new THREE.Vector3(...bed.extents.max),
  );
  frameBox(camera, controls, box);
}
