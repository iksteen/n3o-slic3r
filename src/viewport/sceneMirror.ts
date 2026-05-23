// Local Three.js mirror of the Rust authoritative scene (PR-2-9).
//
// The renderer holds **no** authoritative state per AD-8. This module
// is a passive reflector: events come in via `applyEvent`, the
// Three.js side updates, and the renderer paints whatever the mirror
// currently says. Selection, transforms, gizmo state — all owned by
// Rust.
//
// Tests use the `MeshBufferProvider` callback to inject canned
// buffers without going through Tauri IPC (see __test__/).

import * as THREE from "three";
import type {
  BedMesh,
  CameraState,
  GizmoState,
  MeshHeader,
  MeshId,
  ObjectId,
  SceneEvent,
  SceneObject,
  SceneSnapshot,
} from "./types";

/** Resolver for binary mesh buffers — at runtime this calls Tauri's
 * `scene_mesh_buffers(meshId)` command and decodes the LE-packed
 * [vertices][normals][indices] payload. Pluggable so tests can pass a
 * fake without needing IPC. */
export type MeshBufferProvider = (
  header: MeshHeader,
) => Promise<{ vertices: Float32Array; normals: Float32Array; indices: Uint32Array }>;

/** Outline material for selected objects. Phase 4 swaps this for a
 * proper post-process outline; for MVP we tint the base material. */
const SELECTED_COLOR = 0x3b82f6; // tailwind blue-500
const DEFAULT_COLOR = 0xb1b1b1;

interface ObjectRecord {
  /** Three.js mesh in the scene graph. */
  mesh: THREE.Mesh;
  /** Material reused so selection tinting is reversible. */
  material: THREE.MeshStandardMaterial;
  /** Last-applied serialized data — keeps us from rebuilding on
   * no-op `ObjectUpdated` events (e.g., a rotate that lands the same
   * matrix). Not used by the renderer's display, just for diagnostics. */
  data: SceneObject;
}

interface MeshRecord {
  geometry: THREE.BufferGeometry;
  header: MeshHeader;
}

export class SceneMirror {
  /** Root of objects in the scene graph. The bed + zone overlays live
   * under [`bedGroup`](#bedGroup) so callers can toggle them
   * independently. */
  readonly objectGroup = new THREE.Group();
  readonly bedGroup = new THREE.Group();

  private meshes = new Map<MeshId, MeshRecord>();
  private objects = new Map<ObjectId, ObjectRecord>();
  private selection = new Set<ObjectId>();
  private bufferProvider: MeshBufferProvider;
  private listeners: Array<(e: SceneEvent) => void> = [];
  /** Serialization queue for `applyEvent`. Tauri's event listener
   * fires synchronously per event but `applyEvent("MeshLoaded")`
   * awaits a binary buffer fetch — without a queue, a later
   * `ObjectAdded` could run before its mesh registers, drop on the
   * "unknown mesh" floor, and never appear in the viewport. */
  private queue: Promise<void> = Promise.resolve();

  /** Current camera state from the Rust side. The viewport reads
   * this and (debounced) writes back via `scene_camera_set`. */
  camera: CameraState = {
    position: [200, -200, 200],
    target: [0, 0, 0],
    up: [0, 0, 1],
    fov_degrees: 45,
    projection: "Perspective",
  };
  gizmo: GizmoState = { mode: "None", pivot: null };
  bed: BedMesh | null = null;

  constructor(bufferProvider: MeshBufferProvider) {
    this.bufferProvider = bufferProvider;
    this.objectGroup.name = "n3o:scene-objects";
    this.bedGroup.name = "n3o:bed";
  }

  /** Used by test code to observe what the mirror just applied. The
   * eventBridge emits raw Tauri events; this fires for both raw
   * events and the snapshot-replay path. */
  onEvent(listener: (e: SceneEvent) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  /** Wholesale rebuild from a Rust-side snapshot. Used on first
   * mount + after a renderer reconnect. Mesh buffers are
   * lazy-loaded — the snapshot only carries headers. */
  async applySnapshot(snapshot: SceneSnapshot): Promise<void> {
    this.clear();
    for (const header of snapshot.meshes) {
      await this.applyEvent({ kind: "MeshLoaded", data: header });
    }
    for (const obj of snapshot.objects) {
      await this.applyEvent({ kind: "ObjectAdded", data: obj });
    }
    await this.applyEvent({
      kind: "SelectionChanged",
      data: { selected: snapshot.selection },
    });
    await this.applyEvent({ kind: "CameraChanged", data: snapshot.camera });
    await this.applyEvent({ kind: "GizmoChanged", data: snapshot.gizmo });
    await this.applyEvent({ kind: "BedChanged", data: snapshot.bed });
  }

  /** Apply one event from the Rust side. Events are serialized
   * through an internal queue so a long-running `MeshLoaded` (async
   * buffer fetch) doesn't allow a following `ObjectAdded` to race
   * ahead and miss its mesh in the registry. */
  applyEvent(event: SceneEvent): Promise<void> {
    const next = this.queue.then(() => this.handleEvent(event));
    // Detach errors from the queue so one failed event doesn't
    // permanently jam every subsequent applyEvent.
    this.queue = next.catch((err) => {
      console.error("scene mirror event failed", event, err);
    });
    return next;
  }

  private async handleEvent(event: SceneEvent): Promise<void> {
    switch (event.kind) {
      case "MeshLoaded":
        await this.registerMesh(event.data);
        break;
      case "ObjectAdded":
        this.addObject(event.data);
        break;
      case "ObjectUpdated":
        this.updateObject(event.data);
        break;
      case "ObjectRemoved":
        this.removeObject(event.data.id);
        break;
      case "SelectionChanged":
        this.setSelection(event.data.selected);
        break;
      case "GizmoChanged":
        this.gizmo = event.data;
        break;
      case "CameraChanged":
        this.camera = event.data;
        break;
      case "BedChanged":
        this.applyBed(event.data);
        break;
      case "ObjectOutOfBounds":
        // Non-blocking warning; renderer can flash the object.
        // For MVP we just tint it momentarily, but since the warning
        // doesn't carry a duration we leave the visual cue to the
        // UI layer (a toast). Mirror just notifies listeners.
        break;
      case "NonUniformScale":
      case "AutoArrangeOverflow":
        // Same: pass-through, the UI panel handles these toasts.
        break;
    }
    for (const l of this.listeners) {
      l(event);
    }
  }

  private async registerMesh(header: MeshHeader): Promise<void> {
    if (this.meshes.has(header.id)) {
      // Mesh IDs are monotonic on the Rust side; a duplicate means
      // we're being replayed from snapshot after a reconnect. Reuse
      // the existing geometry rather than fetching the same buffer
      // twice.
      return;
    }
    const buffers = await this.bufferProvider(header);
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(buffers.vertices, 3),
    );
    geometry.setAttribute(
      "normal",
      new THREE.BufferAttribute(buffers.normals, 3),
    );
    geometry.setIndex(new THREE.BufferAttribute(buffers.indices, 1));
    geometry.computeBoundingSphere();
    this.meshes.set(header.id, { geometry, header });
  }

  private addObject(obj: SceneObject): void {
    if (this.objects.has(obj.id)) {
      // Replay path — treat like an update.
      this.updateObject(obj);
      return;
    }
    const meshRec = this.meshes.get(obj.mesh);
    if (!meshRec) {
      console.warn("ObjectAdded references unknown mesh", obj);
      return;
    }
    const material = new THREE.MeshStandardMaterial({
      color: DEFAULT_COLOR,
      metalness: 0.0,
      roughness: 0.8,
    });
    const mesh = new THREE.Mesh(meshRec.geometry, material);
    mesh.name = `obj:${obj.id}`;
    mesh.userData.objectId = obj.id;
    mesh.visible = obj.visible;
    mesh.matrixAutoUpdate = false;
    applyTransform(mesh, obj);
    this.objectGroup.add(mesh);
    this.objects.set(obj.id, { mesh, material, data: obj });
    if (this.selection.has(obj.id)) {
      this.tintForSelection(obj.id, true);
    }
  }

  private updateObject(obj: SceneObject): void {
    const rec = this.objects.get(obj.id);
    if (!rec) {
      this.addObject(obj);
      return;
    }
    if (rec.data.mesh !== obj.mesh) {
      // Mesh swap — rare, but possible if PR-2-7's library re-instances.
      // Rebuild the whole record.
      this.removeObject(obj.id);
      this.addObject(obj);
      return;
    }
    rec.mesh.visible = obj.visible;
    applyTransform(rec.mesh, obj);
    rec.data = obj;
  }

  private removeObject(id: ObjectId): void {
    const rec = this.objects.get(id);
    if (!rec) return;
    this.objectGroup.remove(rec.mesh);
    rec.material.dispose();
    // Geometry is shared via the mesh registry — don't dispose here.
    this.objects.delete(id);
    this.selection.delete(id);
  }

  private setSelection(ids: ObjectId[]): void {
    const next = new Set(ids);
    for (const old of this.selection) {
      if (!next.has(old)) {
        this.tintForSelection(old, false);
      }
    }
    for (const id of next) {
      if (!this.selection.has(id)) {
        this.tintForSelection(id, true);
      }
    }
    this.selection = next;
  }

  private tintForSelection(id: ObjectId, selected: boolean): void {
    const rec = this.objects.get(id);
    if (!rec) return;
    rec.material.color.setHex(selected ? SELECTED_COLOR : DEFAULT_COLOR);
    rec.material.emissive.setHex(selected ? 0x0a1b3a : 0x000000);
  }

  private applyBed(bed: BedMesh | null): void {
    // Reset the bed group.
    while (this.bedGroup.children.length > 0) {
      const child = this.bedGroup.children[0];
      this.bedGroup.remove(child);
      disposeObject3D(child);
    }
    this.bed = bed;
    if (!bed) return;

    const { extents, grid_spacing, exclusion_zones } = bed;
    const minX = extents.min[0];
    const minY = extents.min[1];
    const maxX = extents.max[0];
    const maxY = extents.max[1];
    const z = extents.min[2];

    // Grid lines at every grid_spacing in both X and Y.
    const gridGeo = new THREE.BufferGeometry();
    const points: number[] = [];
    for (let x = minX; x <= maxX + 1e-6; x += grid_spacing) {
      points.push(x, minY, z, x, maxY, z);
    }
    for (let y = minY; y <= maxY + 1e-6; y += grid_spacing) {
      points.push(minX, y, z, maxX, y, z);
    }
    gridGeo.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(points, 3),
    );
    const gridLines = new THREE.LineSegments(
      gridGeo,
      new THREE.LineBasicMaterial({ color: 0x444444, transparent: true, opacity: 0.5 }),
    );
    gridLines.name = "n3o:bed-grid";
    this.bedGroup.add(gridLines);

    // Outline (heavy edge of the bed rectangle).
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
    const outline = new THREE.LineSegments(
      outlineGeo,
      new THREE.LineBasicMaterial({ color: 0x888888 }),
    );
    outline.name = "n3o:bed-outline";
    this.bedGroup.add(outline);

    // Exclusion zones (red wireframe AABBs).
    for (const zone of exclusion_zones) {
      this.bedGroup.add(buildZoneWireframe(zone.bounds, zone.label));
    }
  }

  // ---- Test / inspector accessors -------------------------------------
  hasMesh(id: MeshId): boolean {
    return this.meshes.has(id);
  }
  hasObject(id: ObjectId): boolean {
    return this.objects.has(id);
  }
  selectedIds(): ObjectId[] {
    return Array.from(this.selection).sort((a, b) => a - b);
  }
  objectColor(id: ObjectId): number | null {
    return this.objects.get(id)?.material.color.getHex() ?? null;
  }
  objectMatrix(id: ObjectId): number[] | null {
    const rec = this.objects.get(id);
    return rec ? rec.mesh.matrix.toArray() : null;
  }
  bedChildCount(): number {
    return this.bedGroup.children.length;
  }

  /** Drop every mesh / object / overlay. Used by the snapshot
   * replay path and by Vite hot-reload teardown. */
  clear(): void {
    for (const id of Array.from(this.objects.keys())) {
      this.removeObject(id);
    }
    for (const [, rec] of this.meshes) {
      rec.geometry.dispose();
    }
    this.meshes.clear();
    this.selection.clear();
    while (this.bedGroup.children.length > 0) {
      const child = this.bedGroup.children[0];
      this.bedGroup.remove(child);
      disposeObject3D(child);
    }
    this.bed = null;
  }
}

function applyTransform(mesh: THREE.Mesh, obj: SceneObject): void {
  // `obj.transform.matrix` is column-major 16 floats matching the
  // glam side and THREE.Matrix4.fromArray. Use matrixAutoUpdate=false
  // (set at construction) so this matrix is what the renderer uses
  // verbatim — no risk of Three.js re-deriving from position/quaternion.
  mesh.matrix.fromArray(obj.transform.matrix);
  mesh.matrixWorldNeedsUpdate = true;
}

function buildZoneWireframe(bb: BedMesh["extents"], label: string): THREE.Object3D {
  const w = bb.max[0] - bb.min[0];
  const d = bb.max[1] - bb.min[1];
  const h = Math.max(bb.max[2] - bb.min[2], 1.0);
  const geo = new THREE.BoxGeometry(w, d, h);
  const edges = new THREE.EdgesGeometry(geo);
  geo.dispose();
  const line = new THREE.LineSegments(
    edges,
    new THREE.LineBasicMaterial({ color: 0xef4444, transparent: true, opacity: 0.7 }),
  );
  line.position.set(
    (bb.min[0] + bb.max[0]) * 0.5,
    (bb.min[1] + bb.max[1]) * 0.5,
    (bb.min[2] + bb.max[2]) * 0.5,
  );
  line.name = `n3o:zone:${label}`;
  return line;
}

function disposeObject3D(obj: THREE.Object3D): void {
  obj.traverse((node) => {
    const mesh = node as THREE.Mesh;
    if (mesh.geometry) mesh.geometry.dispose();
    const mat = (mesh as THREE.Mesh).material as
      | THREE.Material
      | THREE.Material[]
      | undefined;
    if (Array.isArray(mat)) {
      for (const m of mat) m.dispose();
    } else if (mat) {
      mat.dispose();
    }
  });
}
