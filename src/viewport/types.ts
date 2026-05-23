// TypeScript mirrors of the Rust scene types (PR-2-9).
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
  parent: ObjectId | null;
}

export type ProjectionMode = "Perspective" | "Orthographic";

export interface CameraState {
  position: [number, number, number];
  target: [number, number, number];
  up: [number, number, number];
  fov_degrees: number;
  projection: ProjectionMode;
}

export type GizmoMode = "None" | "Translate" | "Rotate" | "Scale";

export interface GizmoState {
  mode: GizmoMode;
  pivot: [number, number, number] | null;
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

/** Snapshot returned by the `scene_snapshot` Tauri command. */
export interface SceneSnapshot {
  meshes: MeshHeader[];
  objects: SceneObject[];
  selection: ObjectId[];
  camera: CameraState;
  gizmo: GizmoState;
  plate: unknown | null; // ActivePlate — only used for serialization round-trip; no renderer feature on it yet
  exclusion_zones: ExclusionZone[];
  bed: BedMesh | null;
}

/** Live diff events on `scene:*` channels. Same shape as Rust's
 * `SceneEvent` with `#[serde(tag = "kind", content = "data")]`. */
export type SceneEvent =
  | { kind: "MeshLoaded"; data: MeshHeader }
  | { kind: "ObjectAdded"; data: SceneObject }
  | { kind: "ObjectUpdated"; data: SceneObject }
  | { kind: "ObjectRemoved"; data: { id: ObjectId } }
  | { kind: "SelectionChanged"; data: { selected: ObjectId[] } }
  | { kind: "GizmoChanged"; data: GizmoState }
  | { kind: "CameraChanged"; data: CameraState }
  | { kind: "BedChanged"; data: BedMesh | null }
  | {
      kind: "ObjectOutOfBounds";
      data: { id: ObjectId; reasons: OutOfBoundsReason[] };
    }
  | { kind: "NonUniformScale"; data: { id: ObjectId } }
  | { kind: "AutoArrangeOverflow"; data: { un_placed: ObjectId[] } };
