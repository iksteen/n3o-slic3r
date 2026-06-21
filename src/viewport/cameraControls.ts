// Camera helpers for the viewport (PR-2-9).
//
// Wraps three's `OrbitControls` with `frameBox` / `initialFrameForBed`
// helpers. The viewport uses a single perspective camera — the
// orthographic projection + its toggle were dropped.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { BedMesh } from "./types";

/** Make a camera matching `state`. Bed-side `up` is +Z (mm slicer
 * convention); we don't use Y-up like web defaults.
 *
 * The default offset is a straight-on front-elevated view (no X offset,
 * large −Y, +Z ≈ 38° elevation): with zero azimuth, screen-right is exactly
 * world +X, so the bed's front edge — the X axis — is perfectly horizontal
 * along the bottom, the origin sits at the lower-left corner, and Y recedes
 * symmetrically upward. Any X offset would tilt the X axis (a 45° azimuth
 * renders the square bed as a diamond with the origin at the left vertex).
 * `initialFrameForBed` preserves this direction, so it's the default until
 * the user orbits. */
export function makePerspectiveCamera(aspect: number): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, aspect, 0.5, 5000);
  cam.up.set(0, 0, 1);
  cam.position.set(0, -260, 200);
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

/** Adjust camera + controls so `box` fits the viewport snugly,
 * preserving the current view direction. Module-private — only
 * `initialFrameForBed` uses it.
 *
 * Fits the box's actual projected corners in the current view rather than
 * a rotation-invariant bounding sphere: for each corner we solve the
 * minimum camera distance that keeps it inside the (aspect-corrected,
 * margin-shrunk) frustum, accounting for its depth. This packs a flat
 * plate tightly — a near-axis-aligned view fills the frame instead of
 * leaving the ~30% slack a sphere fit (diagonal) would — while a tilted
 * view still can't clip a corner, since every corner is a constraint. */
function frameBox(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  box: THREE.Box3,
): void {
  if (box.isEmpty()) return;
  const center = new THREE.Vector3();
  box.getCenter(center);

  // View direction (target → camera), preserved from the current pose.
  const dir = camera.position.clone().sub(controls.target).normalize();
  if (dir.lengthSq() < 1e-6) dir.set(0, -1, 0.77).normalize();
  // Camera basis matching three's lookAt: forward (camera → center),
  // screen right, screen up.
  const forward = dir.clone().negate();
  const worldUp = camera.up.clone().normalize();
  const right = new THREE.Vector3().crossVectors(worldUp, dir).normalize();
  if (right.lengthSq() < 1e-6) right.set(1, 0, 0);
  const up = new THREE.Vector3().crossVectors(dir, right).normalize();

  const margin = 1.08; // ~8% breathing room around the tightest corner
  const tanV = Math.tan(((camera.fov * Math.PI) / 180) * 0.5);
  const tanH = tanV * camera.aspect;

  // Minimum distance so every corner stays inside the frustum. For a
  // corner at lateral offset (a,b) and depth offset c (along forward,
  // +ve = behind center → farther → smaller angle), it fits at distance D
  // when a*margin <= (D+c)*tanH and b*margin <= (D+c)*tanV, i.e.
  // D >= a*margin/tanH - c (and the vertical analogue).
  const v = new THREE.Vector3();
  let distance = 0.1;
  for (const cx of [box.min.x, box.max.x]) {
    for (const cy of [box.min.y, box.max.y]) {
      for (const cz of [box.min.z, box.max.z]) {
        v.set(cx, cy, cz).sub(center);
        const a = Math.abs(v.dot(right));
        const b = Math.abs(v.dot(up));
        const c = v.dot(forward);
        distance = Math.max(
          distance,
          (a * margin) / tanH - c,
          (b * margin) / tanV - c,
        );
      }
    }
  }

  camera.position.copy(center).addScaledVector(dir, distance);
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
  // Frame the build *plate* — the footprint at the bottom face — not the
  // whole build volume. `bed.extents` carries the printable height (z up to
  // e.g. 180mm); framing that box centers the camera at mid-volume (z/2) and
  // zooms out to fit the height, leaving the plate small and low in the view.
  // Collapsing z to the plate plane centers + scales on the plate itself, so
  // it stays correct across printers with different height/footprint ratios.
  const [minX, minY, minZ] = bed.extents.min;
  const [maxX, maxY] = bed.extents.max;
  const box = new THREE.Box3(
    new THREE.Vector3(minX, minY, minZ),
    new THREE.Vector3(maxX, maxY, minZ),
  );
  frameBox(camera, controls, box);
}
