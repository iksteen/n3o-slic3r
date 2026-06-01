// Priming-tower overlay.
//
// A translucent prism drawn on the bed at the resolved wipe/prime-tower
// footprint, with a faint brim outline. Position + size come from the
// backend `plate_tower_geometry` command (cascade-resolved, with the
// plate's project overrides folded in, so the box sits exactly where the
// tower will slice). The box is the raycast target for the bed-plane drag
// that ViewportCanvas wires up; dragging writes wipe_tower_x/y project
// overrides, which flow back here on the next resolve.
//
// World space == bed millimetres (the bed's corner is the world origin),
// so geometry values map straight onto positions with no conversion.

import * as THREE from "three";
import type { TowerGeometry } from "./types";

// Indicative height — the real tower is as tall as the print. Tying it to
// the tallest object is a future refinement; a fixed prism already reads
// as a reserved volume and keeps the overlay decoupled from object state.
const TOWER_HEIGHT_MM = 50;
// Blue — distinct from objects (their filament colours) and the red
// exclusion-zone wireframes.
const TOWER_COLOR = 0x3b82f6;

export interface TowerOverlay {
  group: THREE.Group;
  /** The draggable translucent prism — the bed-plane drag raycasts this. */
  box: THREE.Mesh;
  /** Reflect resolved geometry. `bedZ` is the bed's base plane (world Z). */
  update(geom: TowerGeometry, bedZ: number): void;
  setVisible(visible: boolean): void;
  dispose(): void;
}

export function createTowerOverlay(): TowerOverlay {
  const group = new THREE.Group();
  group.name = "n3o:tower";

  // Unit box scaled per-update, so the mesh ref (the drag target) stays
  // stable across geometry changes.
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

  // Crisp edges so the footprint stays legible through the translucency.
  // Parented to the box so the single scale/position update drives both.
  const edges = new THREE.LineSegments(
    new THREE.EdgesGeometry(boxGeo),
    new THREE.LineBasicMaterial({
      color: TOWER_COLOR,
      transparent: true,
      opacity: 0.9,
    }),
  );
  box.add(edges);

  // Brim outline on the bed plane — rebuilt per update.
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

  function update(geom: TowerGeometry, bedZ: number): void {
    const w = Math.max(geom.width, 0.1);
    box.scale.set(w, w, TOWER_HEIGHT_MM);
    box.position.set(
      geom.x + geom.width / 2,
      geom.y + geom.width / 2,
      bedZ + TOWER_HEIGHT_MM / 2,
    );
    box.rotation.z = (geom.rotation * Math.PI) / 180;
    box.updateMatrix();

    // Brim rectangle: the footprint inflated by the brim width, sitting on
    // the bed (lifted a hair to avoid z-fighting with the grid).
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
    brim.visible = b > 0;
  }

  function setVisible(visible: boolean): void {
    group.visible = visible;
  }

  function dispose(): void {
    boxGeo.dispose();
    boxMat.dispose();
    edges.geometry.dispose();
    (edges.material as THREE.Material).dispose();
    brim.geometry.dispose();
    (brim.material as THREE.Material).dispose();
    group.removeFromParent();
  }

  return { group, box, update, setVisible, dispose };
}
