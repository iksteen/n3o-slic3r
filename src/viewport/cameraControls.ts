// Camera helpers for the viewport (PR-2-9).
//
// Wraps three's `OrbitControls` with a `frameAll` helper and a
// perspective ↔ orthographic toggle. The toggle is a cut candidate
// per the ticket but is cheap enough to ship.

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

export function makeOrthographicCamera(
  aspect: number,
  viewSize = 200,
): THREE.OrthographicCamera {
  const half = viewSize * 0.5;
  const cam = new THREE.OrthographicCamera(
    -half * aspect,
    half * aspect,
    half,
    -half,
    0.5,
    5000,
  );
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
 * margin. Works for both perspective + orthographic cameras. */
export function frameBox(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  controls: OrbitControls,
  box: THREE.Box3,
  aspect: number,
): void {
  if (box.isEmpty()) return;
  const size = new THREE.Vector3();
  box.getSize(size);
  const center = new THREE.Vector3();
  box.getCenter(center);

  const maxDim = Math.max(size.x, size.y, size.z) || 1;
  const margin = 1.4;

  if ("isPerspectiveCamera" in camera && (camera as THREE.PerspectiveCamera).isPerspectiveCamera) {
    const persp = camera as THREE.PerspectiveCamera;
    const fov = (persp.fov * Math.PI) / 180;
    const distance = (maxDim * 0.5 * margin) / Math.tan(fov * 0.5);
    // Preserve current view direction by reading the camera's
    // existing offset from the target and rescaling.
    const direction = persp.position.clone().sub(controls.target).normalize();
    if (direction.lengthSq() < 1e-6) {
      direction.set(1, -1, 1).normalize();
    }
    persp.position.copy(center).addScaledVector(direction, distance);
    controls.target.copy(center);
    persp.near = Math.max(distance * 0.01, 0.1);
    persp.far = distance * 100;
    persp.updateProjectionMatrix();
  } else {
    const ortho = camera as THREE.OrthographicCamera;
    const half = maxDim * 0.5 * margin;
    ortho.left = -half * aspect;
    ortho.right = half * aspect;
    ortho.top = half;
    ortho.bottom = -half;
    const direction = ortho.position.clone().sub(controls.target).normalize();
    if (direction.lengthSq() < 1e-6) {
      direction.set(1, -1, 1).normalize();
    }
    ortho.position.copy(center).addScaledVector(direction, maxDim * margin);
    controls.target.copy(center);
    ortho.near = 0.1;
    ortho.far = maxDim * 100;
    ortho.updateProjectionMatrix();
  }
  controls.update();
}

/** Compute a sensible initial view given the bed's extents — pull
 * the camera back along (+X, -Y, +Z) so the user sees the front-left
 * corner. Called before the first frame so there's *something* on
 * screen even when the scene is empty. */
export function initialFrameForBed(
  camera: THREE.PerspectiveCamera | THREE.OrthographicCamera,
  controls: OrbitControls,
  bed: BedMesh,
  aspect: number,
): void {
  const box = new THREE.Box3(
    new THREE.Vector3(...bed.extents.min),
    new THREE.Vector3(...bed.extents.max),
  );
  frameBox(camera, controls, box, aspect);
}
