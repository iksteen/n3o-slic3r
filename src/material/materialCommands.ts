// Tauri invoke wrappers for the per-plate material binding panel
// (PR-5-6 UI).
//
// The backend mutations + commands shipped in `PR-5-6 backend`.
// This module is the thin wrapper layer so callers don't have to
// remember command names + arg keys; the panel + tests use it.

import { invoke } from "@tauri-apps/api/core";
import type { PlateId } from "../viewport/types";

/** Upsert a binding for `(plate, modelMaterial)`. The
 * `physical_slot` is 1-based against the bound printer's
 * `slot_count`; `filament_identity` is the identity slug of a
 * filament profile from the bundled catalog (stubbed in
 * `material/filamentCatalog.ts` until a real registry lands). */
export function setMaterialBinding(
  plateId: PlateId,
  modelMaterial: number,
  physicalSlot: number,
  filamentIdentity: string,
): Promise<void> {
  return invoke("project_set_material_binding", {
    plateId,
    modelMaterial,
    physicalSlot,
    filamentIdentity,
  });
}

/** Drop the binding for `(plate, modelMaterial)`. After this the
 * model material falls back to "use slot 1" at slice time (per
 * the backend's FR-MP-8 contract). The pre-slice gate will
 * refuse the job until the user binds it back. */
export function clearMaterialBinding(
  plateId: PlateId,
  modelMaterial: number,
): Promise<void> {
  return invoke("project_clear_material_binding", {
    plateId,
    modelMaterial,
  });
}
