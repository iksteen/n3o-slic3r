// Objects panel (OP-1) — the left workspace column: the active plate's
// object list with two-way selection sync to the 3D viewport.
//
// Scope: read-only display here. Add/remove (OP-2), per-object material
// editing (OP-3), and grouping (OP-4) layer on later. An object's
// "material" is its `extruder_id`, resolved to a spool colour through
// the plate's `material_to_slot` table + the bound instance's slots —
// the same routing the Materials section of `SlotBindingPanel` uses, so
// the colours agree across both surfaces.

import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PlateSnapshot, SceneObject, ObjectId } from "../viewport/types";
import {
  flattenSlots,
  type FlatSlotOption,
  type PrinterInstance,
} from "../printer/printerInstance";
import {
  addPrimitive,
  createMaterialForObject,
  deleteObject,
  loadModelFromDialog,
  setObjectMaterial,
  PRIMITIVE_KINDS,
} from "./objectCommands";
import { referencedMaterials } from "../material/SlotBindingPanel";
import { useFilamentCatalog } from "../material/useFilamentCatalog";
import { MaterialPicker } from "./MaterialPicker";

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

/** Lowest 1-based material index not yet referenced on the plate, so a
 *  new material reuses a freed gap (e.g. M2 after M2 was removed) rather
 *  than always climbing past the highest. */
function firstAvailableMaterial(used: number[]): number {
  const set = new Set(used);
  let m = 1;
  while (set.has(m)) m++;
  return m;
}

export function ObjectsPanel({
  plate,
  instance,
  printerName,
  plateSize,
  readOnly = false,
}: ObjectsPanelProps) {
  const [showLibrary, setShowLibrary] = useState(false);
  const [materialPicker, setMaterialPicker] = useState<{
    objId: ObjectId;
    rect: DOMRect;
  } | null>(null);
  const { byIdentity: filamentByIdentity } = useFilamentCatalog();

  const slots = useMemo<FlatSlotOption[]>(
    () => (instance ? flattenSlots(instance) : []),
    [instance],
  );
  const materials = useMemo(() => referencedMaterials(plate), [plate]);
  const nextMaterial = firstAvailableMaterial(materials);
  const materialToSlot = plate?.material_to_slot ?? {};

  const onAddPrimitive = (kind: (typeof PRIMITIVE_KINDS)[number]): void => {
    setShowLibrary(false);
    void addPrimitive(kind).catch((err) =>
      console.error("[objects] addPrimitive failed", err),
    );
  };
  const onAddModel = (): void => {
    setShowLibrary(false);
    void loadModelFromDialog().catch((err) =>
      console.error("[objects] loadModelFromDialog failed", err),
    );
  };

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
    // Only show a colour when a filament is actually loaded — a cached
    // spool colour with no identity (e.g. the unloaded external feed)
    // reads as empty, not solid.
    return slot?.filament_identity ? slot.color : null;
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
        {!readOnly && (
          <div className="objects-add">
            <button
              className="objects-add-btn"
              title="Add object"
              aria-label="Add object"
              onClick={() => setShowLibrary((s) => !s)}
            >
              <svg width="13" height="13" viewBox="0 0 14 14" fill="none" aria-hidden>
                <path d="M7 2v10M2 7h10" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
              </svg>
            </button>
            {showLibrary && (
              <>
                <div className="objects-add-backdrop" onClick={() => setShowLibrary(false)} />
                <div className="objects-add-menu" role="menu">
                  <div className="objects-add-section">Primitives</div>
                  {PRIMITIVE_KINDS.map((k) => (
                    <button
                      key={k}
                      className="objects-add-item"
                      role="menuitem"
                      onClick={() => onAddPrimitive(k)}
                    >
                      {k}
                    </button>
                  ))}
                  <div className="objects-add-sep" />
                  <button className="objects-add-item" role="menuitem" onClick={onAddModel}>
                    Add model…
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      <div className="objects-list">
        {objects.length === 0 ? (
          <div className="objects-empty">
            {readOnly
              ? "No objects on this plate."
              : "No objects yet — click + to add one."}
          </div>
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
                        border: color
                          ? "none"
                          : "1px dashed var(--text-muted)",
                      }}
                    />
                    <span className="objects-name-text">{obj.name}</span>
                  </div>
                  <div className="objects-item-meta">
                    {readOnly ? (
                      <span
                        className="objects-material-badge"
                        title={`Material ${material}`}
                      >
                        M{material}
                      </span>
                    ) : (
                      <button
                        className={`objects-material-badge objects-material-badge-btn ${
                          materialPicker?.objId === obj.id ? "open" : ""
                        }`}
                        title={`Material ${material} — click to change`}
                        onClick={(e) => {
                          e.stopPropagation();
                          const rect = e.currentTarget.getBoundingClientRect();
                          setMaterialPicker((p) =>
                            p && p.objId === obj.id ? null : { objId: obj.id, rect },
                          );
                        }}
                      >
                        M{material}
                      </button>
                    )}
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
                {!readOnly && (
                  <button
                    className="objects-remove"
                    title="Remove"
                    aria-label={`Remove ${obj.name}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      void deleteObject(obj.id).catch((err) =>
                        console.error("[objects] deleteObject failed", err),
                      );
                    }}
                  >
                    <svg width="10" height="10" viewBox="0 0 12 12" fill="none" aria-hidden>
                      <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
                    </svg>
                  </button>
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

      {!readOnly &&
        materialPicker &&
        plate &&
        (() => {
          const obj = objects.find((o) => o.id === materialPicker.objId);
          if (!obj) return null;
          return (
            <MaterialPicker
              objectName={obj.name}
              currentMaterial={obj.extruder_id ?? 1}
              materials={materials}
              nextMaterial={nextMaterial}
              slots={slots}
              materialToSlot={materialToSlot}
              filamentByIdentity={filamentByIdentity}
              allowCreate={objects.length > 1}
              anchorRect={materialPicker.rect}
              onAssign={(m) => {
                void setObjectMaterial(obj.id, m).catch((err) =>
                  console.error("[objects] setObjectMaterial failed", err),
                );
              }}
              onCreate={(m, slot) => {
                void createMaterialForObject(
                  plate.plate_id,
                  obj.id,
                  m,
                  slot,
                ).catch((err) =>
                  console.error("[objects] createMaterialForObject failed", err),
                );
              }}
              onClose={() => setMaterialPicker(null)}
            />
          );
        })()}
    </aside>
  );
}
