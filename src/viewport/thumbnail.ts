// Offscreen print-thumbnail render. Produces a clean 3/4 iso PNG of just
// the plate's models (transparent background, no bed/grid/gizmo) for the
// sliced output — the Bambu screen reads it from the `.gcode.3mf` and the
// U1's Klipper UI from a base64 G-code comment block. Independent of the
// live viewport camera: we clone the object group into a throwaway scene,
// frame its bounding box, render once, and read back a PNG.

import * as THREE from "three";

/** Render `source` (the plate's object group) to a square PNG and return
 *  base64 (no `data:` prefix), or `null` when there's nothing to show. */
export function renderModelThumbnail(
  source: THREE.Object3D,
  size = 400,
): string | null {
  // Clone so we never disturb the live scene graph (re-parenting an object
  // would yank it out of the viewport). Geometry is shared by reference;
  // materials are clone-and-cleared just below.
  const root = source.clone(true);
  const scene = new THREE.Scene();
  scene.add(root);
  scene.updateMatrixWorld(true);

  // The clone shares materials by reference, so a *selected* object still
  // carries its selection emissive tint (sceneMirror sets `material.emissive`).
  // Swap in emissive-cleared clones so the preview shows the true material
  // colour, not the on-screen selection glow — without mutating the live
  // materials. Disposed in `finally`.
  const ownedMaterials: THREE.Material[] = [];
  const declink = (m: THREE.Material): THREE.Material => {
    const clone = m.clone();
    const std = clone as THREE.MeshStandardMaterial;
    if (std.emissive) std.emissive.setHex(0x000000);
    ownedMaterials.push(clone);
    return clone;
  };
  root.traverse((obj) => {
    const mesh = obj as THREE.Mesh;
    if (!mesh.isMesh) return;
    mesh.material = Array.isArray(mesh.material)
      ? mesh.material.map(declink)
      : declink(mesh.material);
  });

  const box = new THREE.Box3().setFromObject(root);
  if (box.isEmpty()) return null;
  const center = box.getCenter(new THREE.Vector3());
  const sphere = box.getBoundingSphere(new THREE.Sphere());
  if (sphere.radius <= 0) return null;

  // Lighting mirrors the live viewport (ambient + key/fill) so the preview
  // reads the same as on screen.
  scene.add(new THREE.AmbientLight(0xffffff, 0.55));
  const key = new THREE.DirectionalLight(0xffffff, 0.9);
  key.position.copy(center).add(new THREE.Vector3(150, -150, 250));
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffffff, 0.25);
  fill.position.copy(center).add(new THREE.Vector3(-150, 150, 100));
  scene.add(fill);

  // Iso 3/4 camera framing the bounding sphere; Z-up like the bed frame.
  const fov = 35;
  const camera = new THREE.PerspectiveCamera(fov, 1, 0.1, sphere.radius * 100);
  camera.up.set(0, 0, 1);
  const dir = new THREE.Vector3(1, -1, 0.8).normalize();
  const dist = (sphere.radius / Math.sin((fov / 2) * (Math.PI / 180))) * 1.15;
  camera.position.copy(center).add(dir.multiplyScalar(dist));
  camera.lookAt(center);

  const renderer = new THREE.WebGLRenderer({
    antialias: true,
    alpha: true,
    preserveDrawingBuffer: true,
  });
  try {
    renderer.setSize(size, size);
    renderer.setClearColor(0x000000, 0); // transparent background
    renderer.render(scene, camera);
    const dataUrl = renderer.domElement.toDataURL("image/png");
    // A failed/lost context yields "data:," — reject anything that isn't a
    // real PNG data URL so we never hand the backend un-decodable "base64".
    const prefix = "data:image/png;base64,";
    if (!dataUrl.startsWith(prefix)) return null;
    return dataUrl.slice(prefix.length);
  } finally {
    for (const m of ownedMaterials) m.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  }
}
