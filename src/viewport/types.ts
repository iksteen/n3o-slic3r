// TypeScript mirrors of the Rust scene types (PR-2-9, PR-5-2 phase C).
//
// These exist to give the renderer a typed view of the JSON payloads
// the Tauri commands and `scene:*` events emit. Hand-written rather
// than generated; the Rust side moves slowly enough that one drift
// per phase is cheaper than wiring up `ts-rs` + a build step.
//
// Field names mirror serde's default lowercased layout (Rust field
// `extruder_id` → TS field `extruder_id`). Enums tagged `kind` /
// `data` on the Rust side serialize as `{ kind: "...", data: ... }`
// here — see SceneEvent below.

/** Monotonic mesh id. 1-based on the Rust side. */
export type MeshId = number;

/** Monotonic scene-object id. 1-based on the Rust side. */
export type ObjectId = number;

/** Stable per-plate identifier (PR-5-2). 1-based on the Rust side;
 * never reused even when a plate is removed. */
export type PlateId = number;

/** Column-major [f32; 16] matrix matching glam + THREE.Matrix4.
 *  The Rust `Transform` is `#[serde(transparent)]` over `[f32; 16]`,
 *  so the wire shape is a bare 16-element number array — *not* an
 *  object with a `matrix` field. Keep this typed as a tuple so any
 *  attempt to access `.matrix` is a TypeScript error. */
export type Transform = readonly number[];

export interface BoundingBox {
  min: [number, number, number];
  max: [number, number, number];
}

export type MeshProvenance =
  | { kind: "File"; data: string }
  | { kind: "Primitive"; data: string };

export interface MeshHeader {
  id: MeshId;
  vertex_count: number;
  index_count: number;
  bounding_box: BoundingBox;
  provenance: MeshProvenance;
}

export interface SceneObject {
  id: ObjectId;
  mesh: MeshId;
  transform: Transform;
  name: string;
  visible: boolean;
  extruder_id: number | null;
  /** Group membership — objects sharing a `group_id` are volumes of one
   *  logical object (3MF multi-volume or user grouping). `null` = solo. */
  group_id: number | null;
  parent: ObjectId | null;
}


// Transform-tool mode is renderer-local UI state (not part of the
// backend scene model). It lives here only because the viewport's
// gizmo + toolbar use it. There is no "off" mode — the gizmo simply
// detaches when nothing is selected.
export type GizmoMode = "Translate" | "Rotate" | "Scale";

export interface ExclusionZone {
  label: string;
  bounds: BoundingBox;
}

export interface BedMesh {
  extents: BoundingBox;
  grid_spacing: number;
  origin_marker: [number, number, number];
  exclusion_zones: ExclusionZone[];
}

/** Resolved priming-tower placement + footprint (bed millimetres = world
 *  space). Mirrors the backend `plate_tower_geometry` payload. `x`/`y` are
 *  the tower's lower-left corner, `width` the square footprint, `brim` the
 *  surrounding skirt, `rotation` degrees about the tower. */
export interface TowerGeometry {
  x: number;
  y: number;
  width: number;
  brim: number;
  rotation: number;
  /** Distinct material count this resolved against. A sliced tower mesh is
   *  stale once this diverges from the count it was sliced at (the only
   *  thing that reshapes the tower; moving it does not). */
  material_count: number;
}

/** The prime/wipe tower's exact mesh from a slice — `vertices` is 3 floats
 *  per vertex, `indices` 3 vertex indices per triangle, in tower-local
 *  millimetres (placed at the plate's wipe_tower_x/y). Mirrors the backend
 *  slice-event `tower_mesh` payload. */
export interface TowerMesh {
  vertices: number[];
  indices: number[];
}

export type BoundsAxis = "X" | "Y" | "Z";

export type OutOfBoundsReason =
  | { kind: "OutOfBuildVolume"; data: { axis: BoundsAxis } }
  | { kind: "IntersectsExclusion"; data: { label: string } }
  | { kind: "BelowBuildPlate"; data: null };

// ---- Project-level types (PR-5-1, PR-5-5, PR-5-6) ------------------

/** Per-plate metadata the composition plugin host (Phase 8) reads.
 * `cycle_count` was cut from MVP scope; only `composition_order`
 * survives on the wire. */
export interface PlateMetadata {
  composition_order: number;
}


// ---- Snapshot wire shape (PR-5-2 phase C) --------------------------

/** Per-plate slice of the snapshot. Plate identity + metadata +
 * bindings + scene contents — enough to render a plate tab and
 * its workspace. */
export interface PlateSnapshot {
  // Plate identity / metadata
  plate_id: PlateId;
  name: string;
  metadata: PlateMetadata;
  /** Vendor printer identity derived from the bound
   *  `PrinterInstance.vendor_profile_ref`. Snapshot-only — the
   *  in-memory `Plate` only carries `printer_instance_id`. `null`
   *  for unbound plates or when the bound id no longer resolves. */
  printer_identity: string | null;
  /** PrinterInstance id the plate slices against (PR-S-5c). Sole
   *  carrier of binding state — `null` for an unbound plate.
   *  Drives the slot binding panel + the composer-side cascade
   *  resolution. */
  printer_instance_id: string | null;
  /** Per-plate model material → PrinterInstance slot routing
   *  (PR-S-7). Keyed by 1-based material index; values point into
   *  the bound instance's `(extruder, slot)` grid. Auto-bind
   *  populates this on object register. */
  material_to_slot: Record<number, { extruder: number; slot: number }>;
  project_overrides: Record<string, string>;
  /** The plate's process/quality profile override (a bundled
   *  process-fragment slug), or `null` to inherit the bound instance's
   *  `quality_profile`. Drives the per-plate Quality picker. */
  quality_profile: string | null;

  // Per-plate scene contents
  objects: SceneObject[];
  selection: ObjectId[];
  /** Active build plate identity + transform on this plate (the
   * bed surface selection — distinct from the multi-plate
   * `plate_id` field above). */
  build_plate: unknown | null;
  exclusion_zones: ExclusionZone[];
  bed: BedMesh | null;
  object_overrides: Record<string, Record<string, string>>;
  /** Display names for object groups (`group_id` → name). */
  group_names: Record<number, string>;
}

/** Snapshot returned by the `scene_snapshot` Tauri command (PR-5-2
 * phase C). The renderer rebuilds its mirror from this on first
 * mount + after every reconnect. */
export interface SceneSnapshot {
  project_uuid: string;
  source_path: string | null;
  user_overrides: Record<string, string>;
  file_metadata: Record<string, string>;
  /** Scene-wide mesh registry. Headers only; the renderer follows
   * up per-mesh with `scene_mesh_buffers(id)` for the binary
   * vertex / normal / index data. */
  meshes: MeshHeader[];
  /** All plates in declaration order. */
  plates: PlateSnapshot[];
  /** Stable id of the currently-active plate. */
  active_plate_id: PlateId;
}

/** Live diff events on `scene:*` / `project:*` channels. Matches
 * Rust's `SceneEvent` with `#[serde(tag = "kind", content = "data")]`.
 *
 * **PR-5-2 phase C:** every plate-scoped variant carries `plate_id`
 * as its first data field so the mirror routes the event to the
 * matching per-plate cache. Scene-wide variants (mesh registry,
 * project save/load) stay plate-less. */
export type SceneEvent =
  // ---- Scene-wide ----
  | { kind: "MeshLoaded"; data: { mesh: MeshHeader } }
  // ---- Per-plate scene-graph deltas ----
  | { kind: "ObjectAdded"; data: { plate_id: PlateId; object: SceneObject } }
  | { kind: "ObjectUpdated"; data: { plate_id: PlateId; object: SceneObject } }
  | {
      kind: "ObjectRemoved";
      data: { plate_id: PlateId; object_id: ObjectId };
    }
  | {
      kind: "SelectionChanged";
      data: { plate_id: PlateId; selected: ObjectId[] };
    }
  | {
      kind: "BedChanged";
      data: { plate_id: PlateId; bed: BedMesh | null };
    }
  | {
      kind: "ObjectOutOfBounds";
      data: {
        plate_id: PlateId;
        object_id: ObjectId;
        reasons: OutOfBoundsReason[];
      };
    }
  | {
      kind: "NonUniformScale";
      data: { plate_id: PlateId; object_id: ObjectId };
    }
  | {
      kind: "AutoArrangeOverflow";
      data: { plate_id: PlateId; un_placed: ObjectId[] };
    }
  // ---- Plate list mutations (PR-5-2) ----
  | { kind: "PlateAdded"; data: { plate_id: PlateId } }
  | { kind: "PlateRemoved"; data: { plate_id: PlateId } }
  | { kind: "ActivePlateChanged"; data: { plate_id: PlateId } }
  // ---- Project-state changes (PR-5-5, PR-5-6, PR-5-7) ----
  | { kind: "PlateMetadataChanged"; data: { plate_id: PlateId } }
  | { kind: "MaterialSlotChanged"; data: { plate_id: PlateId } }
  | {
      kind: "ObjectOverridesChanged";
      data: { plate_id: PlateId; object_id: ObjectId };
    }
  // ---- Project save/load (PR-5-8) ----
  | { kind: "ProjectSaved"; data: { path: string } }
  | { kind: "ProjectLoaded"; data: { path: string } };
