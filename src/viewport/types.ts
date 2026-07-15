// TypeScript mirrors of the Rust scene types.
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

/** Stable per-plate identifier. 1-based on the Rust side;
 * never reused even when a plate is removed. */
export type PlateId = number;

/** Stable group identity — a UUID string (Rust `GroupId(Uuid)`). */
export type GroupId = string;

/** Per-group state: display name + the group's cascade overrides.
 *  A group slices as one libslic3r ModelObject, so object-scope
 *  settings (`enable_support`, `layer_height`, …) live here and apply
 *  to every member; members keep only region-scope overrides. The
 *  backend omits `overrides` when empty. */
export interface Group {
  name: string;
  overrides?: Record<string, string>;
}

/** Column-major [f32; 16] matrix matching glam's `Mat4`.
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
  /** Group membership — objects sharing a `group` are volumes of one
   *  logical object (3MF multi-volume or user grouping). `null` = solo.
   *  The group's name lives in `PlateSnapshot.groups`. */
  group: GroupId | null;
}


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

export type BoundsAxis = "X" | "Y" | "Z";

export type OutOfBoundsReason =
  | { kind: "OutOfBuildVolume"; data: { axis: BoundsAxis } }
  | { kind: "IntersectsExclusion"; data: { label: string } }
  | { kind: "BelowBuildPlate"; data: null };

// ---- Snapshot wire shape -------------------------------------------

/** Per-plate slice of the snapshot. Plate identity + bindings +
 * scene contents — enough to render a plate tab and its
 * workspace. */
export interface PlateSnapshot {
  // Plate identity
  plate_id: PlateId;
  name: string;
  /** Vendor printer identity derived from the bound
   *  `PrinterInstance.vendor_profile_ref`. Snapshot-only — the
   *  in-memory `Plate` only carries `printer_instance_id`. `null`
   *  for unbound plates or when the bound id no longer resolves. */
  printer_identity: string | null;
  /** PrinterInstance id the plate slices against. Sole
   *  carrier of binding state — `null` for an unbound plate.
   *  Drives the slot binding panel + the composer-side cascade
   *  resolution. */
  printer_instance_id: string | null;
  /** Per-plate model material → PrinterInstance slot routing.
   *  Keyed by 1-based material index; values point into
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
  /** Per-group state keyed by group id (the display name today). */
  groups: Record<GroupId, Group>;
}

/** Snapshot returned by the `scene_snapshot` Tauri command.
 * The renderer rebuilds its mirror from this on first
 * mount + after every reconnect. */
export interface SceneSnapshot {
  project_uuid: string;
  source_path: string | null;
  user_overrides: Record<string, string>;
  file_metadata: Record<string, string>;
  /** Scene-wide mesh registry — headers only (geometry lives Rust-side and is
   * uploaded straight to the GPU by the wgpu renderer). */
  meshes: MeshHeader[];
  /** All plates in declaration order. */
  plates: PlateSnapshot[];
  /** Stable id of the currently-active plate. */
  active_plate_id: PlateId;
}

/** Live diff events on `scene:*` / `project:*` channels. Matches
 * Rust's `SceneEvent` with `#[serde(tag = "kind", content = "data")]`.
 *
 * Every plate-scoped variant carries `plate_id`
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
      kind: "AutoArrangeOverflow";
      data: { plate_id: PlateId; un_placed: ObjectId[] };
    }
  // ---- Plate list mutations ----
  | { kind: "PlateAdded"; data: { plate_id: PlateId } }
  | { kind: "PlateRemoved"; data: { plate_id: PlateId } }
  | { kind: "ActivePlateChanged"; data: { plate_id: PlateId } }
  // ---- Project-state changes ----
  | { kind: "PlateChanged"; data: { plate_id: PlateId } }
  | { kind: "MaterialSlotChanged"; data: { plate_id: PlateId } }
  | {
      kind: "ObjectOverridesChanged";
      data: { plate_id: PlateId; object_id: ObjectId };
    }
  // ---- Project save/load ----
  | { kind: "ProjectSaved"; data: { path: string } }
  | { kind: "ProjectLoaded"; data: { path: string } };
