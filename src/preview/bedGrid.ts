// Bed grid for the preview scene (PR-6-8).
//
// Reimplements the viewport's bed grid (sceneMirror.ts) here
// rather than extracting it — the viewport version is tied into
// the larger BedMesh / exclusion-zone scaffolding, while the
// preview only needs the rectangle + grid lines. ~40 lines of
// Three.js, not worth the cross-module refactor.

import * as THREE from "three";

import type { BoundingBox } from "./types";

const DEFAULT_GRID_SPACING_MM = 10;

/** Build a bed grid (line segments) for `extents`. Returns a
 * group containing the grid + the outline rectangle. Caller is
 * responsible for adding it to the scene and disposing on
 * unmount (call [`disposeBedGrid`]). */
export function makeBedGrid(extents: BoundingBox): THREE.Group {
  const minX = extents.min[0];
  const minY = extents.min[1];
  const maxX = extents.max[0];
  const maxY = extents.max[1];
  const z = extents.min[2];

  const group = new THREE.Group();
  group.name = "n3o:preview-bed";

  const gridPoints: number[] = [];
  for (
    let x = minX;
    x <= maxX + 1e-6;
    x += DEFAULT_GRID_SPACING_MM
  ) {
    gridPoints.push(x, minY, z, x, maxY, z);
  }
  for (
    let y = minY;
    y <= maxY + 1e-6;
    y += DEFAULT_GRID_SPACING_MM
  ) {
    gridPoints.push(minX, y, z, maxX, y, z);
  }
  const gridGeo = new THREE.BufferGeometry();
  gridGeo.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(gridPoints, 3),
  );
  group.add(
    new THREE.LineSegments(
      gridGeo,
      new THREE.LineBasicMaterial({
        color: 0x444444,
        transparent: true,
        opacity: 0.5,
      }),
    ),
  );

  const outlineGeo = new THREE.BufferGeometry();
  outlineGeo.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(
      [
        minX, minY, z, maxX, minY, z,
        maxX, minY, z, maxX, maxY, z,
        maxX, maxY, z, minX, maxY, z,
        minX, maxY, z, minX, minY, z,
      ],
      3,
    ),
  );
  group.add(
    new THREE.LineSegments(
      outlineGeo,
      new THREE.LineBasicMaterial({ color: 0x888888 }),
    ),
  );

  return group;
}

/** Free the GPU buffers + materials a [`makeBedGrid`] group
 * allocated. Three.js doesn't auto-dispose on `scene.remove()`. */
export function disposeBedGrid(group: THREE.Group): void {
  group.traverse((obj) => {
    if (obj instanceof THREE.LineSegments) {
      obj.geometry.dispose();
      if (Array.isArray(obj.material)) {
        for (const m of obj.material) m.dispose();
      } else {
        obj.material.dispose();
      }
    }
  });
}
