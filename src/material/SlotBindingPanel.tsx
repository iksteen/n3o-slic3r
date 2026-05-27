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
//      `project_set_material_slot`.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PlateId, PlateSnapshot } from "../viewport/types";
import {
  flattenSlots,
  getPrinterInstance,
  setSlotFilament,
  type FlatSlotOption,
  type PrinterInstance,
  type SlotRef,
} from "../printer/printerInstance";
import { SlotChipStrip, type FilamentSummary } from "./SlotChipStrip";

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
  const [filaments, setFilaments] = useState<FilamentSummary[]>([]);

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

  // Pull the bundled vendor filament fragments once per mount. The
  // list is small + stable; no refetch trigger needed until user-
  // library editing lands.
  useEffect(() => {
    let cancelled = false;
    void invoke<FilamentSummary[]>("filament_profile_list")
      .then((list) => {
        if (!cancelled) setFilaments(list);
      })
      .catch((err) =>
        console.error("[slot-binding] filament_profile_list failed", err),
      );
    return () => {
      cancelled = true;
    };
  }, []);

  const slots = useMemo<FlatSlotOption[]>(
    () => (instance ? flattenSlots(instance) : []),
    [instance],
  );

  const materials = useMemo(() => referencedMaterials(plate), [plate]);

  const filamentByIdentity = useMemo(() => {
    const map = new Map<string, FilamentSummary>();
    for (const f of filaments) map.set(f.identity, f);
    return map;
  }, [filaments]);

  if (!plateId || !plate || !instance) {
    return null;
  }

  const onPickFilament = (slot: SlotRef, identity: string | null): void => {
    if (!instance) return;
    void setSlotFilament(instance.id, slot.extruder, slot.slot, identity).catch(
      (err) => console.error("[slot-binding] setSlotFilament failed", err),
    );
  };

  // Placeholder sync action — real driver round-trip lands in 7c-2.
  // Resolves after ~400ms so the SyncSlotsLabel spinner visually
  // fires; the underlying slot state doesn't change until the
  // driver-event listener exists to update it.
  const onSyncSlots = (): Promise<void> =>
    new Promise((r) => setTimeout(r, 400));

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
      {/* Section 1: per-instance slot loadout — horizontal pill
          strip + sync-button-as-row-label. */}
      <SlotChipStrip
        instance={instance}
        slots={slots}
        filaments={filaments}
        onPickFilament={onPickFilament}
        onSync={onSyncSlots}
      />

      {/* Section 2: per-plate material → slot pickers */}
      {materials.length > 0 && (
        <div className="slot-binding-section">
          <div className="slot-binding-head">
            <span className="slot-binding-title">Materials</span>
            <span className="slot-binding-sub">on this plate</span>
          </div>
          {materials.map((mat) => {
            const current = slotForMaterial(mat);
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
                    const filEntry = s.filament_identity
                      ? filamentByIdentity.get(s.filament_identity)
                      : null;
                    const filamentLabel = s.filament_identity
                      ? ` — ${filEntry?.display_name ?? s.filament_identity}`
                      : "";
                    return (
                      <option
                        key={`${s.ref.extruder}-${s.ref.slot}`}
                        value={`${s.ref.extruder}:${s.ref.slot}`}
                      >
                        {s.label}
                        {filamentLabel}
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
