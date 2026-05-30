// Local Three.js mirror of the Rust authoritative Project (PR-2-9,
// rebuilt per-plate in PR-5-2 phase C).
//
// The renderer holds **no** authoritative state per AD-8. This module
// is a passive reflector: events come in via `applyEvent`, the
// Three.js side updates, and the renderer paints whatever the mirror
// currently says. Selection and transforms — all owned by
// Rust.
//
// **Shape (PR-5-2 phase C):**
//   - `SceneMirror` is the project-level root. Holds the scene-wide
//     mesh registry + project metadata + a `Map<PlateId, PlateMirror>`.
//   - `PlateMirror` is one plate's worth of scene state — objects,
//     selection, bed, exclusion zones. Each owns its
//     own internal `THREE.Group` for objects + bed.
//   - The viewport adds `mirror.objectGroup` + `mirror.bedGroup` to
//     its scene. These are stable top-level groups whose **single
//     child** is the active plate's per-plate group. On
//     `ActivePlateChanged`, SceneMirror swaps the child — the viewport
//     never sees the structural change.
//
// Tests use the `MeshBufferProvider` callback to inject canned
// buffers without going through Tauri IPC (see __test__/).

import * as THREE from "three";
import type {
  BedMesh,
  MeshHeader,
  MeshId,
  ObjectId,
  PlateId,
  PlateMetadata,
  PlateSnapshot,
  SceneEvent,
  SceneObject,
  SceneSnapshot,
} from "./types";
import type { PrinterInstance, SlotRef } from "../printer/printerInstance";

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
  /** Resolved spool color (object → material → slot → slot.color),
   * or the neutral default if anything in the chain is unbound. The
   * selection tint overrides this while selected; deselecting
   * restores it. */
  baseColor: number;
  /** Last-applied serialized data — keeps us from rebuilding on
   * no-op `ObjectUpdated` events (e.g., a rotate that lands the same
   * matrix). Not used by the renderer's display, just for diagnostics. */
  data: SceneObject;
}

/** Parse a CSS hex string (`"#ff8800"` or `"ff8800"`) into a Three.js
 * 24-bit number. Returns `null` on malformed input so the caller can
 * fall back to the neutral default. */
function parseHexColor(hex: string | null | undefined): number | null {
  if (!hex) return null;
  const s = hex.startsWith("#") ? hex.slice(1) : hex;
  if (!/^[0-9a-fA-F]{6}$/.test(s)) return null;
  return Number.parseInt(s, 16);
}

interface MeshRecord {
  geometry: THREE.BufferGeometry;
  header: MeshHeader;
}

/** One plate's mirror state. Owns its own object + bed groups; the
 * top-level `SceneMirror` swaps these in/out as the active plate
 * changes. Per-plate metadata + bindings cached here too so the
 * frontend's PlateTabs / settings / binding panels can read them. */
export class PlateMirror {
  readonly plateId: PlateId;
  readonly objectGroup = new THREE.Group();
  readonly bedGroup = new THREE.Group();

  // Plate identity / metadata (PR-5-1, PR-5-5, PR-5-6).
  name: string;
  metadata: PlateMetadata;
  /** Vendor printer identity derived from the bound instance at
   * snapshot time. Surfaced for the picker chip + the cascade
   * preview's printer-profile lookup. `null` for unbound plates. */
  printerIdentity: string | null;
  /** PrinterInstance id this plate slices against (PR-S-5c). The
   * mirror caches it so the spool-color resolver can find the
   * bound instance in `SceneMirror.printerInstances` without going
   * back through a snapshot fetch. */
  printerInstanceId: string | null;
  /** Per-plate model material → PrinterInstance slot routing
   * (PR-S-7). Drives the spool-color paint per object. */
  materialToSlot: Record<number, SlotRef>;
  projectOverrides: Record<string, string>;
  objectOverrides: Record<string, Record<string, string>>;

  // Per-plate scene state.
  objects = new Map<ObjectId, ObjectRecord>();
  selection = new Set<ObjectId>();
  bed: BedMesh | null = null;

  constructor(plateId: PlateId, snap?: PlateSnapshot) {
    this.plateId = plateId;
    this.objectGroup.name = `n3o:plate-${plateId}:objects`;
    this.bedGroup.name = `n3o:plate-${plateId}:bed`;
    this.name = snap?.name ?? `Plate ${plateId}`;
    this.metadata = snap?.metadata ?? { composition_order: plateId };
    this.printerIdentity = snap?.printer_identity ?? null;
    this.printerInstanceId = snap?.printer_instance_id ?? null;
    this.materialToSlot = snap?.material_to_slot ?? {};
    this.projectOverrides = snap?.project_overrides ?? {};
    this.objectOverrides = snap?.object_overrides ?? {};
  }

  /** Dispose every Three.js resource this plate owns. Called when
   * the plate is removed from the project or the whole mirror is
   * cleared. */
  dispose(): void {
    for (const rec of this.objects.values()) {
      rec.material.dispose();
      // Geometry lives in the scene-wide mesh registry; not ours
      // to dispose.
      this.objectGroup.remove(rec.mesh);
    }
    this.objects.clear();
    this.selection.clear();
    disposeGroupChildren(this.bedGroup);
    this.bed = null;
  }
}

export class SceneMirror {
  /** Stable top-level groups the viewport adds to its scene. The
   * single child of each is the active plate's per-plate group.
   * On `ActivePlateChanged` we swap that child — the viewport
   * doesn't have to track the swap. */
  readonly objectGroup = new THREE.Group();
  readonly bedGroup = new THREE.Group();

  // Project-level state (PR-5-1, PR-5-8).
  projectUuid: string | null = null;
  sourcePath: string | null = null;
  userOverrides: Record<string, string> = {};
  fileMetadata: Record<string, string> = {};

  private meshes = new Map<MeshId, MeshRecord>();
  private plates = new Map<PlateId, PlateMirror>();
  /** Scene-wide cache of every PrinterInstance the renderer has been
   * told about, keyed by instance id. Drives the spool-color paint —
   * each plate looks up its bound instance here when resolving an
   * object's `(extruder_id → slot → color)` chain. Populated by
   * `applyPrinterInstance` (bridge calls it on startup with each
   * known instance + on every `printer:instance_changed` event). */
  private printerInstances = new Map<string, PrinterInstance>();
  /** Insertion order tracked separately so `plateOrder()` can return
   * plates in declaration order without leaning on Map iteration. */
  private plateOrderList: PlateId[] = [];
  private activePlateId: PlateId | null = null;
  private bufferProvider: MeshBufferProvider;
  private listeners: Array<(e: SceneEvent) => void> = [];
  /** Serialization queue for `applyEvent`. Tauri's event listener
   * fires synchronously per event but `applyEvent("MeshLoaded")`
   * awaits a binary buffer fetch — without a queue, a later
   * `ObjectAdded` could run before its mesh registers, drop on the
   * "unknown mesh" floor, and never appear in the viewport. */
  private queue: Promise<void> = Promise.resolve();

  constructor(bufferProvider: MeshBufferProvider) {
    this.bufferProvider = bufferProvider;
    this.objectGroup.name = "n3o:scene-objects";
    this.bedGroup.name = "n3o:bed";
  }

  /** Used by test code (and the PR-5-3 frontend tab strip) to observe
   * applied events. The eventBridge emits raw Tauri events; this
   * fires for both raw events and the snapshot-replay path. */
  onEvent(listener: (e: SceneEvent) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter((l) => l !== listener);
    };
  }

  // ---- Active-plate accessors -----------------------------------
  //
  // The viewport / camera / gizmo code reads these to render the
  // currently-active plate. Each falls back to a sensible default
  // when no plate is active (pre-snapshot bootstrap). Avoid
  // throwing — the renderer should keep painting even in the
  // brief window between mirror construction and snapshot apply.

  get bed(): BedMesh | null {
    return this.activePlate()?.bed ?? null;
  }

  activePlate(): PlateMirror | null {
    return this.activePlateId !== null
      ? this.plates.get(this.activePlateId) ?? null
      : null;
  }

  activePlateIdOrNull(): PlateId | null {
    return this.activePlateId;
  }

  plate(id: PlateId): PlateMirror | null {
    return this.plates.get(id) ?? null;
  }

  /** Plates in declaration order — drives PlateTabs UI ordering. */
  plateOrder(): PlateId[] {
    return [...this.plateOrderList];
  }

  // ---- Snapshot + event entry points ----------------------------

  /** Wholesale rebuild from a Rust-side snapshot. Used on first
   * mount + after a renderer reconnect / `ProjectLoaded`. Mesh
   * buffers are lazy-loaded — the snapshot only carries headers. */
  async applySnapshot(snapshot: SceneSnapshot): Promise<void> {
    this.clear();

    this.projectUuid = snapshot.project_uuid;
    this.sourcePath = snapshot.source_path;
    this.userOverrides = { ...snapshot.user_overrides };
    this.fileMetadata = { ...snapshot.file_metadata };

    // Mesh registry first so per-plate objects find their geometry
    // when registered below.
    for (const header of snapshot.meshes) {
      await this.registerMesh(header);
    }

    // Plates, in declaration order. Each PlateMirror constructor
    // captures the snapshot's metadata + bindings +
    // project_overrides + object_overrides. We then
    // synthesize per-object adds + selection + bed events so the
    // Three.js scene graph populates.
    for (const plateSnap of snapshot.plates) {
      const plate = new PlateMirror(plateSnap.plate_id, plateSnap);
      this.plates.set(plateSnap.plate_id, plate);
      this.plateOrderList.push(plateSnap.plate_id);
      for (const obj of plateSnap.objects) {
        this.addObjectOnPlate(plate, obj);
      }
      this.setSelectionOnPlate(plate, plateSnap.selection);
      this.applyBedOnPlate(plate, plateSnap.bed);
    }

    this.setActivePlate(snapshot.active_plate_id);
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
        await this.registerMesh(event.data.mesh);
        break;
      case "ObjectAdded": {
        const plate = this.requirePlate(event.data.plate_id, "ObjectAdded");
        if (plate) this.addObjectOnPlate(plate, event.data.object);
        break;
      }
      case "ObjectUpdated": {
        const plate = this.requirePlate(event.data.plate_id, "ObjectUpdated");
        if (plate) this.updateObjectOnPlate(plate, event.data.object);
        break;
      }
      case "ObjectRemoved": {
        const plate = this.requirePlate(event.data.plate_id, "ObjectRemoved");
        if (plate) this.removeObjectOnPlate(plate, event.data.object_id);
        break;
      }
      case "SelectionChanged": {
        const plate = this.requirePlate(
          event.data.plate_id,
          "SelectionChanged",
        );
        if (plate) this.setSelectionOnPlate(plate, event.data.selected);
        break;
      }
      case "BedChanged": {
        const plate = this.requirePlate(event.data.plate_id, "BedChanged");
        if (plate) this.applyBedOnPlate(plate, event.data.bed);
        break;
      }
      case "ObjectOutOfBounds":
      case "NonUniformScale":
      case "AutoArrangeOverflow":
        // Non-blocking warnings; the UI layer (toast) handles these.
        // The mirror just notifies listeners.
        break;
      case "PlateAdded":
        this.addPlate(event.data.plate_id);
        break;
      case "PlateRemoved":
        this.removePlate(event.data.plate_id);
        break;
      case "ActivePlateChanged":
        this.setActivePlate(event.data.plate_id);
        break;
      case "PlateMetadataChanged":
      case "MaterialSlotChanged":
      case "ObjectOverridesChanged":
        // The mirror keeps a copy of these for fast UI render, but
        // since the canonical source is the project snapshot, the
        // simplest correct path is to no-op here and let the UI
        // re-fetch via the snapshot command when it needs fresh
        // metadata. PR-5-3+ may add inline updates if profiling
        // shows the snapshot fetch is on the hot path.
        break;
      case "ProjectSaved":
        this.sourcePath = event.data.path;
        break;
      case "ProjectLoaded":
        // The frontend should re-fetch the snapshot and call
        // applySnapshot — the new project's plates / meshes /
        // overrides have nothing in common with the prior state.
        break;
    }
    for (const l of this.listeners) {
      l(event);
    }
  }

  // ---- Scene-wide mesh registry ---------------------------------

  private async registerMesh(header: MeshHeader): Promise<void> {
    if (this.meshes.has(header.id)) {
      // Mesh IDs are monotonic on the Rust side; a duplicate means
      // we're being replayed from snapshot after a reconnect. Reuse
      // the existing geometry rather than fetching the same buffer
      // twice.
      return;
    }
    const buffers = await this.bufferProvider(header);
    if (import.meta.env.DEV) {
      console.debug("[n3o] mesh buffers", header.id, {
        vertexCount: buffers.vertices.length / 3,
        indexCount: buffers.indices.length,
      });
    }
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

  // ---- Per-plate scene-graph mutation ---------------------------

  private requirePlate(id: PlateId, eventName: string): PlateMirror | null {
    const plate = this.plates.get(id);
    if (!plate) {
      console.warn(
        `[n3o] ${eventName} for unknown plate ${id}; event dropped`,
      );
      return null;
    }
    return plate;
  }

  private addObjectOnPlate(plate: PlateMirror, obj: SceneObject): void {
    if (plate.objects.has(obj.id)) {
      // Replay path — treat like an update.
      this.updateObjectOnPlate(plate, obj);
      return;
    }
    const meshRec = this.meshes.get(obj.mesh);
    if (!meshRec) {
      console.warn("ObjectAdded references unknown mesh", obj);
      return;
    }
    if (import.meta.env.DEV) {
      console.debug(
        "[n3o] add object",
        obj.id,
        "plate",
        plate.plateId,
        "mesh",
        obj.mesh,
        "tx",
        obj.transform.slice(12, 15),
      );
    }
    const baseColor = this.colorForObject(plate, obj);
    const material = new THREE.MeshStandardMaterial({
      color: baseColor,
      metalness: 0.0,
      roughness: 0.8,
    });
    const mesh = new THREE.Mesh(meshRec.geometry, material);
    mesh.name = `obj:${obj.id}`;
    mesh.userData.objectId = obj.id;
    mesh.userData.plateId = plate.plateId;
    mesh.visible = obj.visible;
    // Leave matrixAutoUpdate=true (Three.js default) so that the
    // PR-2-10 gizmo's drag — which writes to position/quaternion/
    // scale — actually moves the mesh. applyTransform decomposes
    // the incoming column-major matrix into those three so the
    // next-frame recompute reproduces the same matrix.
    applyTransform(mesh, obj);
    plate.objectGroup.add(mesh);
    plate.objects.set(obj.id, { mesh, material, baseColor, data: obj });
    if (plate.selection.has(obj.id)) {
      this.tintForSelection(plate, obj.id, true);
    }
  }

  private updateObjectOnPlate(plate: PlateMirror, obj: SceneObject): void {
    const rec = plate.objects.get(obj.id);
    if (!rec) {
      this.addObjectOnPlate(plate, obj);
      return;
    }
    if (rec.data.mesh !== obj.mesh) {
      // Mesh swap — rare, but possible if PR-2-7's library re-instances.
      // Rebuild the whole record.
      this.removeObjectOnPlate(plate, obj.id);
      this.addObjectOnPlate(plate, obj);
      return;
    }
    rec.mesh.visible = obj.visible;
    applyTransform(rec.mesh, obj);
    // If the model-material assignment changed, the spool-color
    // chain (material → slot → color) may resolve to a different
    // hex. Recompute + repaint (preserving the selection tint).
    if (rec.data.extruder_id !== obj.extruder_id) {
      rec.baseColor = this.colorForObject(plate, obj);
      if (!plate.selection.has(obj.id)) {
        rec.material.color.setHex(rec.baseColor);
      }
    }
    rec.data = obj;
  }

  private removeObjectOnPlate(plate: PlateMirror, id: ObjectId): void {
    const rec = plate.objects.get(id);
    if (!rec) return;
    plate.objectGroup.remove(rec.mesh);
    rec.material.dispose();
    // Geometry is shared via the mesh registry — don't dispose here.
    plate.objects.delete(id);
    plate.selection.delete(id);
  }

  private setSelectionOnPlate(plate: PlateMirror, ids: ObjectId[]): void {
    const next = new Set(ids);
    for (const old of plate.selection) {
      if (!next.has(old)) {
        this.tintForSelection(plate, old, false);
      }
    }
    for (const id of next) {
      if (!plate.selection.has(id)) {
        this.tintForSelection(plate, id, true);
      }
    }
    plate.selection = next;
  }

  private tintForSelection(
    plate: PlateMirror,
    id: ObjectId,
    selected: boolean,
  ): void {
    const rec = plate.objects.get(id);
    if (!rec) return;
    // Selection overrides the spool color with a uniform blue so
    // the picked object stands out regardless of its slot's tint.
    // Deselect restores `baseColor`, not a global default — that's
    // how each object keeps its own spool color across selection
    // cycles.
    rec.material.color.setHex(selected ? SELECTED_COLOR : rec.baseColor);
    rec.material.emissive.setHex(selected ? 0x0a1b3a : 0x000000);
  }

  private applyBedOnPlate(plate: PlateMirror, bed: BedMesh | null): void {
    disposeGroupChildren(plate.bedGroup);
    plate.bed = bed;
    if (!bed) return;
    buildBedOverlay(plate.bedGroup, bed);
  }

  // ---- Spool-color resolution (PR-S-7) --------------------------
  //
  // The render color for an object is `obj.extruder_id →
  // plate.materialToSlot → slot.color`. Anything unbound along the
  // chain falls back to `DEFAULT_COLOR` so an early-bootstrap plate
  // (no printer yet) or an out-of-range slot still renders.

  /** Resolve the spool color for one object on one plate, falling
   * back to `DEFAULT_COLOR` if any link in the chain is missing.
   * Pure — no side effects, no mutation. */
  private colorForObject(plate: PlateMirror, obj: SceneObject): number {
    const material = obj.extruder_id ?? 1;
    const slot = plate.materialToSlot[material];
    if (!slot) return DEFAULT_COLOR;
    if (!plate.printerInstanceId) return DEFAULT_COLOR;
    const inst = this.printerInstances.get(plate.printerInstanceId);
    if (!inst) return DEFAULT_COLOR;
    const ext = inst.extruders[slot.extruder];
    if (!ext) return DEFAULT_COLOR;
    const slotBinding = ext.slots[slot.slot];
    if (!slotBinding) return DEFAULT_COLOR;
    return parseHexColor(slotBinding.color) ?? DEFAULT_COLOR;
  }

  /** Re-resolve every object's baseColor on this plate + repaint
   * (preserving any selection tint). Called when anything upstream
   * of the resolver changes (instance updated, material→slot map
   * updated). */
  private recolorPlate(plate: PlateMirror): void {
    for (const [id, rec] of plate.objects) {
      const next = this.colorForObject(plate, rec.data);
      if (next === rec.baseColor) continue;
      rec.baseColor = next;
      if (!plate.selection.has(id)) {
        rec.material.color.setHex(next);
      }
    }
  }

  /** Public: cache a `PrinterInstance` snapshot + recolor every
   * plate currently bound to it. Bridge calls this on startup with
   * each bundled instance + on every `printer:instance_changed`
   * event with the fresh post-mutation instance. */
  applyPrinterInstance(instance: PrinterInstance): void {
    this.printerInstances.set(instance.id, instance);
    for (const plate of this.plates.values()) {
      if (plate.printerInstanceId === instance.id) {
        this.recolorPlate(plate);
      }
    }
  }

  /** Public: replace a plate's routing inputs (printer instance +
   * material→slot map) + recolor. Bridge calls this whenever an
   * event fires that may have mutated either: `MaterialSlotChanged`,
   * `ObjectAdded`, `PlateMetadataChanged` (printer swap). Both
   * fields are needed because a printer swap changes the instance
   * id, not just the map. */
  applyPlateRouting(
    plateId: PlateId,
    printerInstanceId: string | null,
    materialToSlot: Record<number, SlotRef>,
  ): void {
    const plate = this.plates.get(plateId);
    if (!plate) return;
    plate.printerInstanceId = printerInstanceId;
    plate.materialToSlot = materialToSlot;
    this.recolorPlate(plate);
  }

  // ---- Plate list mutations -------------------------------------

  private addPlate(id: PlateId): void {
    if (this.plates.has(id)) return;
    this.plates.set(id, new PlateMirror(id));
    this.plateOrderList.push(id);
  }

  private removePlate(id: PlateId): void {
    const plate = this.plates.get(id);
    if (!plate) return;
    if (this.activePlateId === id) {
      this.objectGroup.remove(plate.objectGroup);
      this.bedGroup.remove(plate.bedGroup);
      this.activePlateId = null;
    }
    plate.dispose();
    this.plates.delete(id);
    this.plateOrderList = this.plateOrderList.filter((p) => p !== id);
  }

  private setActivePlate(id: PlateId): void {
    if (this.activePlateId === id) return;
    const prior = this.activePlateId !== null
      ? this.plates.get(this.activePlateId) ?? null
      : null;
    const next = this.plates.get(id);
    if (!next) {
      console.warn(`[n3o] ActivePlateChanged: unknown plate ${id}`);
      return;
    }
    if (prior) {
      this.objectGroup.remove(prior.objectGroup);
      this.bedGroup.remove(prior.bedGroup);
    }
    this.objectGroup.add(next.objectGroup);
    this.bedGroup.add(next.bedGroup);
    this.activePlateId = id;
  }

  // ---- Test / inspector accessors -------------------------------
  //
  // These read the **active plate** by default so existing
  // single-plate tests keep working without rewrites. Multi-plate
  // tests use the explicit per-plate variants.

  hasMesh(id: MeshId): boolean {
    return this.meshes.has(id);
  }
  hasObject(id: ObjectId): boolean {
    return this.activePlate()?.objects.has(id) ?? false;
  }
  hasObjectOnPlate(plateId: PlateId, id: ObjectId): boolean {
    return this.plates.get(plateId)?.objects.has(id) ?? false;
  }
  selectedIds(): ObjectId[] {
    const sel = this.activePlate()?.selection;
    return sel ? Array.from(sel).sort((a, b) => a - b) : [];
  }
  objectColor(id: ObjectId): number | null {
    return this.activePlate()?.objects.get(id)?.material.color.getHex() ?? null;
  }
  objectMatrix(id: ObjectId): number[] | null {
    const rec = this.activePlate()?.objects.get(id);
    return rec ? rec.mesh.matrix.toArray() : null;
  }
  bedChildCount(): number {
    return this.activePlate()?.bedGroup.children.length ?? 0;
  }

  /** Look up the Three.js mesh for an object on the active plate.
   * Used by the gizmo to attach to the selected object. Returns
   * `null` when the object isn't on the active plate (or doesn't
   * exist at all). */
  findActiveMesh(id: ObjectId): THREE.Mesh | null {
    return this.activePlate()?.objects.get(id)?.mesh ?? null;
  }

  /** Drop every plate / mesh / overlay. Used by snapshot replay,
   * Vite hot-reload teardown, and `ProjectLoaded`. */
  clear(): void {
    for (const plate of this.plates.values()) {
      plate.dispose();
    }
    // Detach the active plate's groups from the top-level groups.
    while (this.objectGroup.children.length > 0) {
      this.objectGroup.remove(this.objectGroup.children[0]);
    }
    while (this.bedGroup.children.length > 0) {
      this.bedGroup.remove(this.bedGroup.children[0]);
    }
    this.plates.clear();
    this.plateOrderList = [];
    this.activePlateId = null;
    for (const rec of this.meshes.values()) {
      rec.geometry.dispose();
    }
    this.meshes.clear();
    this.projectUuid = null;
    this.sourcePath = null;
    this.userOverrides = {};
    this.fileMetadata = {};
  }
}

function applyTransform(mesh: THREE.Mesh, obj: SceneObject): void {
  // `obj.transform` is column-major 16 floats matching the glam
  // side and THREE.Matrix4.fromArray (Rust's Transform is
  // `#[serde(transparent)]` over `[f32; 16]`, so the wire shape is
  // a bare array). Decompose into position/quaternion/scale so the
  // gizmo's drag (which writes to those three) actually moves the
  // mesh — matrixAutoUpdate=true (default) recomposes the matrix
  // next frame. Our matrices are built from TRS compositions on
  // the Rust side so they decompose cleanly even with non-uniform
  // scale or mirror.
  const m = new THREE.Matrix4().fromArray(obj.transform as number[]);
  m.decompose(mesh.position, mesh.quaternion, mesh.scale);
  // Sync the matrix immediately from P/Q/S so callers that read
  // `mesh.matrix` (tests, raycaster, gizmo attach) see the final
  // value without waiting for the next render frame.
  mesh.updateMatrix();
}

function buildBedOverlay(group: THREE.Group, bed: BedMesh): void {
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
    new THREE.LineBasicMaterial({
      color: 0x444444,
      transparent: true,
      opacity: 0.5,
    }),
  );
  gridLines.name = "n3o:bed-grid";
  group.add(gridLines);

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
  group.add(outline);

  // Exclusion zones (red wireframe AABBs).
  for (const zone of exclusion_zones) {
    group.add(buildZoneWireframe(zone.bounds, zone.label));
  }
}

function buildZoneWireframe(
  bb: BedMesh["extents"],
  label: string,
): THREE.Object3D {
  const w = bb.max[0] - bb.min[0];
  const d = bb.max[1] - bb.min[1];
  const h = Math.max(bb.max[2] - bb.min[2], 1.0);
  const geo = new THREE.BoxGeometry(w, d, h);
  const edges = new THREE.EdgesGeometry(geo);
  geo.dispose();
  const line = new THREE.LineSegments(
    edges,
    new THREE.LineBasicMaterial({
      color: 0xef4444,
      transparent: true,
      opacity: 0.7,
    }),
  );
  line.position.set(
    (bb.min[0] + bb.max[0]) * 0.5,
    (bb.min[1] + bb.max[1]) * 0.5,
    (bb.min[2] + bb.max[2]) * 0.5,
  );
  line.name = `n3o:zone:${label}`;
  return line;
}

function disposeGroupChildren(group: THREE.Group): void {
  while (group.children.length > 0) {
    const child = group.children[0];
    group.remove(child);
    disposeObject3D(child);
  }
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
