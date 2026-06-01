// Priming-tower overlay.
//
// Two representations, both placed on the bed at the resolved tower
// position (world space == bed millimetres, so geometry maps straight to
// positions):
//   • a predicted translucent box (footprint × an indicative height) —
//     shown before a slice and while dragging, when the exact shape isn't
//     known;
//   • the exact mesh libslic3r built during the last slice (a box for AMS
//     purge towers, the rib/cone solid for toolchangers) — shown once a
//     slice provides it, and kept across drags (it's just re-placed) until
//     a material-count change makes it stale.
//
// The box is the bed-plane drag target before a slice; the real mesh is
// the target once shown. `dragTarget()` returns whichever is live.

import * as THREE from "three";
import type { TowerGeometry, TowerMesh } from "./types";

// Indicative height for the *predicted* box — the real tower is as tall as
// the print, which we only know once sliced (the real mesh carries it).
const TOWER_HEIGHT_MM = 50;
// Blue — distinct from objects (filament colours) and red exclusion zones.
const TOWER_COLOR = 0x3b82f6;

type Mode = "box" | "mesh" | "hidden";

export interface TowerOverlay {
  group: THREE.Group;
  /** The mesh the bed-plane drag should raycast — whichever representation
   *  is currently shown, or null when hidden. */
  dragTarget(): THREE.Mesh | null;
  /** Show the predicted box at `geom`. */
  showBox(geom: TowerGeometry, bedZ: number): void;
  /** Show the exact sliced mesh, placed at `geom`'s corner. */
  showMesh(mesh: TowerMesh, geom: TowerGeometry, bedZ: number): void;
  /** Re-place the currently-shown representation at `geom` (e.g. a drag),
   *  without rebuilding geometry. */
  place(geom: TowerGeometry, bedZ: number): void;
  /** The footprint extent (incl. brim) of the *real* sliced mesh, in
   *  tower-local mm relative to the placement corner — `null` unless the
   *  mesh is shown. The drag uses it to clamp the true (width × depth)
   *  footprint on-bed; the predicted box falls back to width × width. */
  meshFootprint(): { minX: number; minY: number; maxX: number; maxY: number } | null;
  hide(): void;
  dispose(): void;
}

export function createTowerOverlay(): TowerOverlay {
  const group = new THREE.Group();
  group.name = "n3o:tower";
  let mode: Mode = "hidden";

  // ---- Predicted box (unit cube scaled per-update; stable ref) --------
  const boxGeo = new THREE.BoxGeometry(1, 1, 1);
  const boxMat = new THREE.MeshStandardMaterial({
    color: TOWER_COLOR,
    transparent: true,
    opacity: 0.28,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const box = new THREE.Mesh(boxGeo, boxMat);
  box.name = "n3o:tower-box";
  box.userData.tower = true;
  group.add(box);
  const boxEdges = new THREE.LineSegments(
    new THREE.EdgesGeometry(boxGeo),
    new THREE.LineBasicMaterial({
      color: TOWER_COLOR,
      transparent: true,
      opacity: 0.9,
    }),
  );
  box.add(boxEdges);
  const brim = new THREE.LineSegments(
    new THREE.BufferGeometry(),
    new THREE.LineBasicMaterial({
      color: TOWER_COLOR,
      transparent: true,
      opacity: 0.4,
    }),
  );
  brim.name = "n3o:tower-brim";
  group.add(brim);

  // ---- Exact sliced mesh (geometry rebuilt on showMesh) ---------------
  const realMat = new THREE.MeshStandardMaterial({
    color: TOWER_COLOR,
    transparent: true,
    opacity: 0.45,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  const realMesh = new THREE.Mesh(new THREE.BufferGeometry(), realMat);
  realMesh.name = "n3o:tower-mesh";
  realMesh.userData.tower = true;
  realMesh.visible = false;
  group.add(realMesh);

  function setBoxVisible(visible: boolean): void {
    box.visible = visible;
    brim.visible = visible && (brim.geometry.getAttribute("position")?.count ?? 0) > 0;
  }

  function placeBox(geom: TowerGeometry, bedZ: number): void {
    const w = Math.max(geom.width, 0.1);
    box.scale.set(w, w, TOWER_HEIGHT_MM);
    box.position.set(
      geom.x + geom.width / 2,
      geom.y + geom.width / 2,
      bedZ + TOWER_HEIGHT_MM / 2,
    );
    box.rotation.z = (geom.rotation * Math.PI) / 180;
    box.updateMatrix();
    const b = geom.brim;
    const x0 = geom.x - b;
    const y0 = geom.y - b;
    const x1 = geom.x + geom.width + b;
    const y1 = geom.y + geom.width + b;
    const z = bedZ + 0.05;
    brim.geometry.dispose();
    const g = new THREE.BufferGeometry();
    g.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(
        [
          x0, y0, z, x1, y0, z,
          x1, y0, z, x1, y1, z,
          x1, y1, z, x0, y1, z,
          x0, y1, z, x0, y0, z,
        ],
        3,
      ),
    );
    brim.geometry = g;
  }

  function placeMesh(geom: TowerGeometry, bedZ: number): void {
    // The mesh is in tower-local millimetres; the tower's world corner is
    // (wipe_tower_x, wipe_tower_y) on the bed plane.
    realMesh.position.set(geom.x, geom.y, bedZ);
    realMesh.rotation.z = (geom.rotation * Math.PI) / 180;
    realMesh.updateMatrix();
  }

  return {
    group,
    dragTarget() {
      if (mode === "mesh") return realMesh;
      if (mode === "box") return box;
      return null;
    },
    showBox(geom, bedZ) {
      mode = "box";
      placeBox(geom, bedZ);
      setBoxVisible(true);
      realMesh.visible = false;
      group.visible = true;
    },
    showMesh(mesh, geom, bedZ) {
      mode = "mesh";
      const g = new THREE.BufferGeometry();
      g.setAttribute(
        "position",
        new THREE.Float32BufferAttribute(Float32Array.from(mesh.vertices), 3),
      );
      g.setIndex(mesh.indices);
      g.computeVertexNormals();
      g.computeBoundingBox();
      realMesh.geometry.dispose();
      realMesh.geometry = g;
      placeMesh(geom, bedZ);
      realMesh.visible = true;
      setBoxVisible(false);
      group.visible = true;
    },
    place(geom, bedZ) {
      if (mode === "mesh") placeMesh(geom, bedZ);
      else if (mode === "box") placeBox(geom, bedZ);
    },
    meshFootprint() {
      if (mode !== "mesh") return null;
      const bb = realMesh.geometry.boundingBox;
      if (!bb) return null;
      return { minX: bb.min.x, minY: bb.min.y, maxX: bb.max.x, maxY: bb.max.y };
    },
    hide() {
      mode = "hidden";
      group.visible = false;
    },
    dispose() {
      boxGeo.dispose();
      boxMat.dispose();
      boxEdges.geometry.dispose();
      (boxEdges.material as THREE.Material).dispose();
      brim.geometry.dispose();
      (brim.material as THREE.Material).dispose();
      realMesh.geometry.dispose();
      realMat.dispose();
      group.removeFromParent();
    },
  };
}
