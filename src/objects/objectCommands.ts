// Object-mutation commands for the Objects panel: add a primitive, load
// a mesh from a file, remove/group/recolour an object. Thin invoke
// wrappers — the backend owns the scene; the panel re-renders off the
// snapshot it refetches on the emitted scene events.

import { invoke } from "@tauri-apps/api/core";
import { openFile } from "../ui/fileDialog";
import type { ObjectId, MeshId, PlateId, GroupId } from "../viewport/types";
import type { SlotRef } from "../printer/printerInstance";

/** The five library primitives — must match Rust `PrimitiveKind`. */
export type PrimitiveKind = "Cube" | "Cylinder" | "Sphere" | "Cone" | "Torus";

export const PRIMITIVE_KINDS: readonly PrimitiveKind[] = [
  "Cube",
  "Cylinder",
  "Sphere",
  "Cone",
  "Torus",
];

/** Select an object (replacing the current selection). */
async function selectObject(id: ObjectId): Promise<void> {
  await invoke("scene_select", { ids: [id], mode: "Replace" });
}

/** Add a primitive at its backend-default size, then select it. */
export async function addPrimitive(kind: PrimitiveKind): Promise<ObjectId> {
  // params omitted → the backend fills in `defaults_for(kind)`.
  const [, objId] = await invoke<[MeshId, ObjectId]>(
    "scene_object_add_from_primitive",
    { kind },
  );
  await selectObject(objId);
  return objId;
}

/** Load a model file's geometry onto the active plate. `.stl`/`.obj`
 *  load a single mesh (then select it); `.3mf` loads only the geometry
 *  (objects + transforms + per-part extruder hints) via
 *  `scene_load_3mf` — no settings (use `loadModelWithSettingsFromDialog`
 *  for those, or "open project" for a full import). */
export async function loadModelFromPath(path: string): Promise<void> {
  if (path.toLowerCase().endsWith(".3mf")) {
    // Geometry-only import — multiple objects, no single one to select.
    await invoke("scene_load_3mf", { path });
    return;
  }
  const [, objId] = await invoke<[MeshId, ObjectId]>(
    "scene_load_mesh_from_path",
    { path },
  );
  await selectObject(objId);
}

/** Pick a model file via the native dialog and load it. Mirrors the
 *  viewport's load button. Cancelling is a no-op. */
export async function loadModelFromDialog(): Promise<void> {
  const path = await openFile({
    filters: [{ name: "Model", extensions: ["stl", "obj", "3mf"] }],
  });
  if (path == null) return; // cancelled
  await loadModelFromPath(path);
}

/** Pick a `.3mf` and load its models WITH their settings: the source
 *  project's config (diffed against the current plate, object-applicable
 *  keys only) applies to every imported object, each object's own
 *  settings merged over it. Filament/machine settings never come along —
 *  n3o owns the printer and the filament. Cancelling is a no-op. */
export async function loadModelWithSettingsFromDialog(): Promise<void> {
  const path = await openFile({
    filters: [{ name: "3MF project", extensions: ["3mf"] }],
  });
  if (path == null) return; // cancelled
  await invoke("scene_load_3mf", { path, importSettings: true });
}

/** Remove one object from the active plate. */
export async function deleteObject(id: ObjectId): Promise<void> {
  await invoke("scene_object_delete", { ids: [id] });
}

/** Clone the given objects on the active plate. `copies` is the number of
 *  copies to make (stacked in place); pass `null` for "fill plate" — clone
 *  the set until the next copy would need another plate, packing with
 *  `arrange` (the clone panel's nester options; omitted → backend
 *  defaults). Per-object settings and group structure are duplicated with
 *  the geometry. `expandGroups` (like the orient/align tools) expands
 *  `ids` to whole groups first, so cloning a single picked volume clones
 *  its siblings. Returns the new object ids. */
export async function cloneObjects(
  ids: ObjectId[],
  copies: number | null,
  expandGroups = false,
  arrange?: { spacing_mm: number; allow_rotations: boolean },
): Promise<ObjectId[]> {
  return invoke<ObjectId[]>("scene_object_clone", {
    ids,
    copies,
    expandGroups,
    spacingMm: arrange?.spacing_mm ?? null,
    allowRotations: arrange?.allow_rotations ?? null,
  });
}

/** Assign an existing material (1-based) to an object. The backend
 *  auto-binds the material to a slot if it had none. */
export async function setObjectMaterial(
  id: ObjectId,
  material: number,
): Promise<void> {
  await invoke("scene_set_object_material", { id, material });
}

/** Mint a new material, route it to `slot`, then assign it to the
 *  object — reusing the existing material→slot routing. */
export async function createMaterialForObject(
  plateId: PlateId,
  id: ObjectId,
  material: number,
  slot: SlotRef,
): Promise<void> {
  await invoke("project_set_material_slot", {
    plateId,
    modelMaterial: material,
    slot,
  });
  await invoke("scene_set_object_material", { id, material });
}

/** Move a set of objects from one plate to another, keeping their
 *  authored XYZ (the "Send to plate" action). Whole groups move
 *  together and the moved materials' slot bindings follow. */
export async function moveObjectsToPlate(
  fromPlate: PlateId,
  toPlate: PlateId,
  ids: ObjectId[],
): Promise<void> {
  await invoke("scene_move_objects_to_plate", {
    fromPlate,
    toPlate,
    objectIds: ids,
  });
}

/** Group objects into one logical (multi-volume) object on the active
 *  plate. No-op for fewer than two ids (enforced backend-side). */
export async function groupObjects(
  ids: ObjectId[],
  name: string,
): Promise<void> {
  await invoke("scene_group_objects", { ids, name });
}

/** Ungroup a group (clear its members' group). */
export async function ungroupObjects(group: GroupId): Promise<void> {
  await invoke("scene_ungroup_objects", { group });
}

/** Rename a group. */
export async function renameGroup(
  group: GroupId,
  name: string,
): Promise<void> {
  await invoke("scene_rename_group", { group, name });
}
