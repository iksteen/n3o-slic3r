// Per-plate slot binding panel (PR-S-7 restoration of MaterialBindingPanel).
//
// Two sections, both writing into the active plate's bound
// PrinterInstance + the plate's `material_to_slot` map:
//
//   1. **Slots** (per-instance) — every (extruder, slot) tuple
//      with a filament picker. Writes via
//      `printer_instance_set_slot_filament`. Shared across every
//      plate that references this instance.
//
//   2. **Materials** (per-plate) — every model material referenced
//      by an object on this plate with a slot picker. Writes via
//      `project_set_material_slot`. Slot options grey out when
//      picking would create a Bambu feed-mix conflict (Direct +
//      Ams on the same extruder).

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PlateId, PlateSnapshot } from "../viewport/types";
import {
  flattenSlots,
  getPrinterInstance,
  isFeedMixConflict,
  setSlotFilament,
  type FlatSlotOption,
  type PrinterInstance,
  type SlotRef,
} from "../printer/printerInstance";
import { FILAMENT_CATALOG, lookupFilament } from "./filamentCatalog";

export interface SlotBindingPanelProps {
  plateId: PlateId | null;
  plate: PlateSnapshot | null;
}

/** The 1-based model material indices referenced by objects on the
 *  plate. Sorted ascending. Exported for tests. */
export function referencedMaterials(plate: PlateSnapshot | null): number[] {
  if (!plate) return [];
  const seen = new Set<number>();
  for (const obj of plate.objects) {
    seen.add(obj.extruder_id ?? 1);
  }
  return Array.from(seen).sort((a, b) => a - b);
}

async function setMaterialSlot(
  plateId: PlateId,
  modelMaterial: number,
  slot: SlotRef,
): Promise<void> {
  await invoke("project_set_material_slot", {
    plateId,
    modelMaterial,
    slot,
  });
}

async function clearMaterialSlot(
  plateId: PlateId,
  modelMaterial: number,
): Promise<void> {
  await invoke("project_clear_material_slot", { plateId, modelMaterial });
}

export function SlotBindingPanel({ plateId, plate }: SlotBindingPanelProps) {
  const instanceId = plate?.printer_instance_id ?? null;
  const [instance, setInstance] = useState<PrinterInstance | null>(null);

  // Pull the live instance state. Refetches whenever the bound id
  // changes, or when the backend emits `printer:instance_changed`
  // (e.g. another panel writes a slot binding).
  useEffect(() => {
    if (!instanceId) {
      setInstance(null);
      return;
    }
    let cancelled = false;
    void getPrinterInstance(instanceId).then((inst) => {
      if (!cancelled) setInstance(inst);
    });
    let unlisten: UnlistenFn | null = null;
    void listen<string>("printer:instance_changed", (event) => {
      if (event.payload !== instanceId) return;
      void getPrinterInstance(instanceId).then((inst) => {
        if (!cancelled) setInstance(inst);
      });
    }).then((u) => {
      if (cancelled) u();
      else unlisten = u;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [instanceId]);

  const slots = useMemo<FlatSlotOption[]>(
    () => (instance ? flattenSlots(instance) : []),
    [instance],
  );

  const materials = useMemo(() => referencedMaterials(plate), [plate]);

  if (!plateId || !plate || !instance) {
    return null;
  }

  const onPickFilament = (slot: SlotRef, identity: string | null): void => {
    if (!instance) return;
    void setSlotFilament(instance.id, slot.extruder, slot.slot, identity).catch(
      (err) => console.error("[slot-binding] setSlotFilament failed", err),
    );
  };

  // Find the FlatSlotOption that matches the plate's current
  // material→slot pick (if any).
  const slotForMaterial = (material: number): FlatSlotOption | null => {
    const pick = plate.material_to_slot?.[material];
    if (!pick) return null;
    return (
      slots.find(
        (s) => s.ref.extruder === pick.extruder && s.ref.slot === pick.slot,
      ) ?? null
    );
  };

  const otherSlotsInUse = (excludeMaterial: number): FlatSlotOption[] => {
    const used: FlatSlotOption[] = [];
    for (const m of materials) {
      if (m === excludeMaterial) continue;
      const pick = slotForMaterial(m);
      if (pick) used.push(pick);
    }
    return used;
  };

  const onPickMaterialSlot = (material: number, slot: SlotRef): void => {
    void setMaterialSlot(plateId, material, slot).catch((err) =>
      console.error("[slot-binding] setMaterialSlot failed", err),
    );
  };

  const onClearMaterialSlot = (material: number): void => {
    void clearMaterialSlot(plateId, material).catch((err) =>
      console.error("[slot-binding] clearMaterialSlot failed", err),
    );
  };

  return (
    <div className="slot-binding-panel">
      {/* Section 1: per-instance slot → filament pickers */}
      <div className="slot-binding-section">
        <div className="slot-binding-head">
          <span className="slot-binding-title">Slots</span>
          <span className="slot-binding-sub">
            {instance.display_name}
          </span>
        </div>
        {slots.map((s) => {
          const entry = s.filament_identity
            ? lookupFilament(s.filament_identity)
            : null;
          const swatch = entry?.color ?? "#9CA3AF";
          return (
            <div
              key={`slot-${s.ref.extruder}-${s.ref.slot}`}
              className="slot-binding-row"
            >
              <span className="slot-binding-label">{s.label}</span>
              <span
                className="slot-binding-swatch"
                style={{ background: swatch }}
                aria-hidden
              />
              <select
                className="slot-binding-filament"
                value={s.filament_identity ?? ""}
                onChange={(e) =>
                  onPickFilament(s.ref, e.target.value || null)
                }
                title={`Filament loaded in ${s.label}`}
              >
                <option value="">— empty —</option>
                {FILAMENT_CATALOG.map((f) => (
                  <option key={f.identity} value={f.identity}>
                    {f.label}
                  </option>
                ))}
                {s.filament_identity && !entry && (
                  // Saved instance may carry an identity not in the
                  // stub catalog — surface verbatim so the user can
                  // still see what's bound.
                  <option value={s.filament_identity}>
                    {s.filament_identity}
                  </option>
                )}
              </select>
            </div>
          );
        })}
      </div>

      {/* Section 2: per-plate material → slot pickers */}
      {materials.length > 0 && (
        <div className="slot-binding-section">
          <div className="slot-binding-head">
            <span className="slot-binding-title">Materials</span>
            <span className="slot-binding-sub">on this plate</span>
          </div>
          {materials.map((mat) => {
            const current = slotForMaterial(mat);
            const conflictWith = otherSlotsInUse(mat);
            const currentValue = current
              ? `${current.ref.extruder}:${current.ref.slot}`
              : "";
            return (
              <div key={`mat-${mat}`} className="slot-binding-row">
                <span className="slot-binding-index">M{mat}</span>
                <span className="slot-binding-arrow" aria-hidden>
                  →
                </span>
                <select
                  className="slot-binding-slot"
                  value={currentValue}
                  onChange={(e) => {
                    if (!e.target.value) {
                      onClearMaterialSlot(mat);
                      return;
                    }
                    const [eIdx, sIdx] = e.target.value
                      .split(":")
                      .map((v) => Number.parseInt(v, 10));
                    onPickMaterialSlot(mat, { extruder: eIdx, slot: sIdx });
                  }}
                  title="Physical slot on the bound printer"
                >
                  <option value="">slot…</option>
                  {slots.map((s) => {
                    const conflict = isFeedMixConflict(s, conflictWith);
                    const filamentLabel = s.filament_identity
                      ? ` — ${s.filament_identity}`
                      : "";
                    return (
                      <option
                        key={`${s.ref.extruder}-${s.ref.slot}`}
                        value={`${s.ref.extruder}:${s.ref.slot}`}
                        disabled={conflict}
                      >
                        {s.label}
                        {filamentLabel}
                        {conflict ? " (feed conflict)" : ""}
                      </option>
                    );
                  })}
                </select>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
