// Per-plate material binding panel (PR-5-6 UI).
//
// Lists every model-material index referenced by objects on the
// active plate, with a dropdown to pick the physical slot + the
// loaded filament identity.
//
// The panel reads the referenced-material set from the active
// plate's `objects[i].extruder_id` (1-based, null = "default
// extruder = 1"). Existing bindings come from
// `plate.material_bindings`. Slot count comes from the bound
// printer profile (or 1 if no printer is bound yet — the panel
// degrades gracefully).
//
// The backend's `Project::register_object` auto-binds each newly
// introduced model material to slot `((mat-1) MOD slot_count)+1`
// with the stub catalog's default filament ("Generic PLA"), so
// the panel typically lands with every row pre-bound; the user
// edits this panel to override the defaults.
//
// Validation pressure (FR-MP-8) is the backend's job — the
// `start_slice_job` gate refuses jobs with unbound materials.
// This panel just surfaces the picker.

import { useMemo } from "react";
import type {
  MaterialBinding,
  PlateId,
  PlateSnapshot,
} from "../viewport/types";
import {
  clearMaterialBinding,
  setMaterialBinding,
} from "./materialCommands";
import { FILAMENT_CATALOG, lookupFilament } from "./filamentCatalog";

export interface MaterialBindingPanelProps {
  /** Active plate id, or `null` before the snapshot lands. */
  plateId: PlateId | null;
  /** Active plate snapshot (objects + material bindings). `null`
   * disables the panel; we render an empty hint. */
  plate: PlateSnapshot | null;
  /** Slot count from the bound printer profile. Defaults to 1
   * when the plate has no printer yet — the picker still works
   * (with a single available slot) so the user can bind in
   * preparation for a printer assignment. */
  slotCount: number;
}

/** Compute the unique model-material indices referenced by
 * objects on the plate. `extruder_id` is the per-object material
 * pointer (1-based; `null` defaults to slot 1, matching the
 * libslic3r convention). Sorted ascending so the panel renders
 * in a predictable order. Exported for tests. */
export function referencedMaterials(plate: PlateSnapshot | null): number[] {
  if (!plate) return [];
  const seen = new Set<number>();
  for (const obj of plate.objects) {
    seen.add(obj.extruder_id ?? 1);
  }
  return Array.from(seen).sort((a, b) => a - b);
}

/** Pull the binding for `modelMaterial`, or `null` when unbound. */
function bindingFor(
  plate: PlateSnapshot | null,
  modelMaterial: number,
): MaterialBinding | null {
  if (!plate) return null;
  return (
    plate.material_bindings.find((b) => b.model_material === modelMaterial) ??
    null
  );
}

export function MaterialBindingPanel({
  plateId,
  plate,
  slotCount,
}: MaterialBindingPanelProps) {
  const referenced = useMemo(() => referencedMaterials(plate), [plate]);

  if (plateId === null || plate === null) {
    return null;
  }

  // No model materials referenced yet — the plate has no objects
  // or every object defaults to slot 1 without bindings. Don't
  // render the panel header; it's noise until there's something
  // to bind.
  if (referenced.length === 0) {
    return null;
  }

  const handleSlotChange = (modelMaterial: number, slot: number): void => {
    const existing = bindingFor(plate, modelMaterial);
    const filament =
      existing?.filament_identity ?? FILAMENT_CATALOG[0]?.identity ?? "";
    void setMaterialBinding(plateId, modelMaterial, slot, filament).catch(
      (err) => console.error("[material] setMaterialBinding failed", err),
    );
  };

  const handleFilamentChange = (
    modelMaterial: number,
    filamentIdentity: string,
  ): void => {
    const existing = bindingFor(plate, modelMaterial);
    const slot = existing?.physical_slot ?? 1;
    void setMaterialBinding(plateId, modelMaterial, slot, filamentIdentity).catch(
      (err) => console.error("[material] setMaterialBinding failed", err),
    );
  };

  const handleClear = (modelMaterial: number): void => {
    void clearMaterialBinding(plateId, modelMaterial).catch((err) =>
      console.error("[material] clearMaterialBinding failed", err),
    );
  };

  return (
    <div className="material-binding-panel">
      <div className="material-binding-head">
        <span className="material-binding-title">Materials</span>
      </div>
      {referenced.map((mat) => {
        const binding = bindingFor(plate, mat);
        const filamentEntry = binding
          ? lookupFilament(binding.filament_identity)
          : null;
        const swatchColor = filamentEntry?.color ?? "#9CA3AF";
        const isBound = binding !== null;
        return (
          <div
            key={mat}
            className={`material-binding-row${isBound ? "" : " unbound"}`}
          >
            <span className="material-binding-index">M{mat}</span>
            <select
              className="material-binding-slot"
              value={binding?.physical_slot ?? ""}
              onChange={(e) => handleSlotChange(mat, Number(e.target.value))}
              title="Physical slot on the bound printer"
            >
              {!isBound && <option value="">slot…</option>}
              {Array.from({ length: slotCount }, (_, i) => i + 1).map((slot) => (
                <option key={slot} value={slot}>
                  Slot {slot}
                </option>
              ))}
            </select>
            <span className="material-binding-arrow" aria-hidden>
              →
            </span>
            <span
              className="material-binding-swatch"
              style={{ background: swatchColor }}
              aria-hidden
            />
            <select
              className="material-binding-filament"
              value={binding?.filament_identity ?? ""}
              onChange={(e) => handleFilamentChange(mat, e.target.value)}
              title="Loaded filament"
            >
              {!isBound && <option value="">filament…</option>}
              {FILAMENT_CATALOG.map((f) => (
                <option key={f.identity} value={f.identity}>
                  {f.label}
                </option>
              ))}
              {binding && !filamentEntry && (
                // Saved project may carry an identity that's not in
                // our stub catalog — surface it as a verbatim option
                // so the panel can still display the bound state.
                <option value={binding.filament_identity}>
                  {binding.filament_identity}
                </option>
              )}
            </select>
            {isBound && (
              <button
                type="button"
                className="material-binding-clear"
                onClick={() => handleClear(mat)}
                title="Clear binding"
                aria-label={`Clear material ${mat} binding`}
              >
                ×
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}
