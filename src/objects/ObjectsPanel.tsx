// Objects panel (OP-1) — the left workspace column: the active plate's
// object list with two-way selection sync to the 3D viewport.
//
// Scope: read-only display here. Add/remove (OP-2), per-object material
// editing (OP-3), and grouping (OP-4) layer on later. An object's
// "material" is its `extruder_id`, resolved to a spool colour through
// the plate's `material_to_slot` table + the bound instance's slots —
// the same routing the Materials section of `SlotBindingPanel` uses, so
// the colours agree across both surfaces.

import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PlateSnapshot, SceneObject, ObjectId } from "../viewport/types";
import {
  flattenSlots,
  type FlatSlotOption,
  type PrinterInstance,
} from "../printer/printerInstance";

export interface ObjectsPanelProps {
  plate: PlateSnapshot | null;
  instance: PrinterInstance | null;
  printerName: string;
  /** Build-plate [x, y] mm for the footer; null when unbound. */
  plateSize: [number, number] | null;
  /** Preview mode: selection still works for cross-reference, but the
   *  panel presents read-only (which it already is in OP-1). */
  readOnly?: boolean;
}

/** An object's material index — its `extruder_id`, defaulting to 1
 *  (unassigned inherits material 1), matching `referencedMaterials`. */
function materialOf(obj: SceneObject): number {
  return obj.extruder_id ?? 1;
}

export function ObjectsPanel({
  plate,
  instance,
  printerName,
  plateSize,
  readOnly = false,
}: ObjectsPanelProps) {
  const slots = useMemo<FlatSlotOption[]>(
    () => (instance ? flattenSlots(instance) : []),
    [instance],
  );

  const objects = plate?.objects ?? [];
  const selection = useMemo(
    () => new Set(plate?.selection ?? []),
    [plate?.selection],
  );

  // Material index → routed slot's spool colour (CSS hex), via the
  // plate's material_to_slot table + the bound instance's slots.
  const colorForMaterial = (material: number): string | null => {
    const pick = plate?.material_to_slot?.[material];
    if (!pick) return null;
    const slot = slots.find(
      (s) => s.ref.extruder === pick.extruder && s.ref.slot === pick.slot,
    );
    return slot?.color ?? null;
  };

  const overrideCount = (id: ObjectId): number =>
    Object.keys(plate?.object_overrides?.[String(id)] ?? {}).length;

  const onSelect = (id: ObjectId, additive: boolean): void => {
    void invoke("scene_select", {
      ids: [id],
      mode: additive ? "Add" : "Replace",
    }).catch((err) => console.error("[objects] scene_select failed", err));
  };

  return (
    <aside className={`objects-panel ${readOnly ? "readonly" : ""}`}>
      <div className="objects-panel-head">
        <h3 title={printerName}>
          Plate · <span className="objects-panel-printer">{printerName}</span>
        </h3>
      </div>

      <div className="objects-list">
        {objects.length === 0 ? (
          <div className="objects-empty">No objects on this plate.</div>
        ) : (
          objects.map((obj) => {
            const material = materialOf(obj);
            const color = colorForMaterial(material);
            const selected = selection.has(obj.id);
            const overrides = overrideCount(obj.id);
            const x = Math.round(obj.transform[12] ?? 0);
            const y = Math.round(obj.transform[13] ?? 0);
            return (
              <div
                key={obj.id}
                role="button"
                tabIndex={0}
                className={`objects-item ${selected ? "selected" : ""}`}
                onClick={(e) =>
                  onSelect(obj.id, e.metaKey || e.ctrlKey || e.shiftKey)
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onSelect(obj.id, e.metaKey || e.ctrlKey || e.shiftKey);
                  }
                }}
              >
                <div className="objects-item-main">
                  <div className="objects-item-name">
                    <span
                      className="objects-color-tag"
                      style={{
                        background: color ?? "transparent",
                        border: color ? "none" : "1px dashed currentColor",
                      }}
                    />
                    <span className="objects-name-text">{obj.name}</span>
                  </div>
                  <div className="objects-item-meta">
                    <span
                      className="objects-material-badge"
                      title={`Material ${material}`}
                    >
                      M{material}
                    </span>
                    <span className="dim">
                      {x}, {y} mm
                    </span>
                  </div>
                </div>
                {overrides > 0 && (
                  <span
                    className="objects-overrides"
                    title={`${overrides} per-object override${overrides === 1 ? "" : "s"}`}
                  >
                    {overrides}
                  </span>
                )}
              </div>
            );
          })
        )}
      </div>

      <div className="objects-panel-foot">
        <div className="objects-stat">
          <span className="k">Build plate</span>
          <span className="v">
            {plateSize
              ? `${Math.round(plateSize[0])} × ${Math.round(plateSize[1])} mm`
              : "—"}
          </span>
        </div>
        <div className="objects-stat">
          <span className="k">Objects</span>
          <span className="v">{objects.length}</span>
        </div>
      </div>
    </aside>
  );
}
