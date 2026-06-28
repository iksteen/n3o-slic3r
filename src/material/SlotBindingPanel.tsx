// Per-plate slot binding panel (restored from MaterialBindingPanel).
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

import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PlateId, PlateSnapshot } from "../viewport/types";
import {
  setSlotColor,
  setSlotFilament,
  type FlatSlotOption,
  type SlotRef,
} from "../printer/printerInstance";
import { usePrinterInstance } from "../printer/usePrinterInstance";
import type { DriverId } from "../driver/types";
import { MaterialChip } from "./MaterialChip";
import { boundMaterials, isRfidDetected, slotForMaterial } from "./materials";
import { SlotChipStrip } from "./SlotChipStrip";
import { useFilamentCatalog } from "./useFilamentCatalog";

export interface SlotBindingPanelProps {
  plateId: PlateId | null;
  plate: PlateSnapshot | null;
  /** Live driver id for the bound printer instance, owned by
   *  `useDriverConnections` and threaded through App.tsx →
   *  SettingsPanelHost. `null` when the bound printer has no
   *  usable connection; the Sync button rejects with the
   *  error-triangle state in that case. */
  driverId: DriverId | null;
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

export function SlotBindingPanel({ plateId, plate, driverId }: SlotBindingPanelProps) {
  const instanceId = plate?.printer_instance_id ?? null;
  // Live instance state via the shared per-instance query — refetches on
  // `printer:instance_changed` for this id (e.g. another panel writes a slot
  // binding) and shares the fetch with the settings host, which reads the
  // same id.
  const instance = usePrinterInstance(instanceId);
  const { list: filaments, byIdentity: filamentByIdentity } =
    useFilamentCatalog();

  const slots: FlatSlotOption[] = instance?.slots ?? [];

  const materials = useMemo(() => boundMaterials(plate), [plate]);

  /** Object count per material (×N badge on each chip). */
  const useCountByMaterial = useMemo(() => {
    const counts = new Map<number, number>();
    if (!plate) return counts;
    for (const obj of plate.objects) {
      const m = obj.extruder_id ?? 1;
      counts.set(m, (counts.get(m) ?? 0) + 1);
    }
    return counts;
  }, [plate]);


  if (!plateId || !plate || !instance) {
    return null;
  }

  // Push the just-edited slot's identity back to the printer (Bambu
  // AMS lite). The edit *is* the trigger — no separate button. Gated to
  // an editable AMS-feed slot on a connected driver; RFID slots are
  // non-editable upstream (belt-and-suspenders here), and a missing
  // driver simply skips the push (the local binding already persisted).
  const pushSlotToAms = async (slot: SlotRef): Promise<void> => {
    if (!instance || driverId == null) return;
    const opt = slots.find(
      (s) => s.ref.extruder === slot.extruder && s.ref.slot === slot.slot,
    );
    if (!opt || opt.feed !== "ams" || isRfidDetected(opt.tag_uid)) return;
    await invoke("driver_ams_set_filament", {
      driverId,
      instanceId: instance.id,
      extruderIdx: slot.extruder,
      slotIdx: slot.slot,
    });
  };

  const onApplyPick = (
    slot: SlotRef,
    pick: { identity: string; color: string },
  ): void => {
    if (!instance) return;
    // Two backend writes — the second runs after the first resolves
    // so a fast-emitted `printer:instance_changed` between them
    // doesn't show a half-updated state. Either failing is logged
    // but we still try the other. Once both persist, auto-push the new
    // identity to the AMS (the backend re-reads the freshly-written
    // slot, so the push reflects this edit).
    void setSlotFilament(instance.id, slot.extruder, slot.slot, pick.identity)
      .catch((err) =>
        console.error("[slot-binding] setSlotFilament failed", err),
      )
      .then(() =>
        setSlotColor(instance.id, slot.extruder, slot.slot, pick.color).catch(
          (err) => console.error("[slot-binding] setSlotColor failed", err),
        ),
      )
      .then(() =>
        pushSlotToAms(slot).catch((err) =>
          console.error("[slot-binding] driver_ams_set_filament failed", err),
        ),
      );
  };

  // Manual sync — pulls the printer's current spool loadout into
  // the instance via the live driver. The driverId
  // prop is the reactive snapshot from `useDriverConnections`,
  // threaded through SettingsPanelHost. Rejects when no driver
  // is registered or when the backend command errors;
  // SyncSlotsLabel turns that into the error-triangle state.
  const onSyncSlots = async (): Promise<void> => {
    if (!instance) throw new Error("no instance");
    if (driverId == null) {
      throw new Error("printer not connected");
    }
    await invoke("printer_instance_sync_from_driver", {
      instanceId: instance.id,
      driverId,
    });
  };

  // FlatSlotOption matching the plate's current material→slot pick.
  const slotFor = (material: number): FlatSlotOption | null =>
    slotForMaterial(material, plate.material_to_slot ?? {}, slots);

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
        onApplyPick={onApplyPick}
        onSync={onSyncSlots}
      />

      {/* Section 2: per-plate material → slot pickers. NOZZLES-
          style divider, then one MaterialChip per referenced
          material. */}
      {materials.length > 0 && (
        <>
          <div
            className="sp-config-divider"
            role="separator"
            aria-label="Materials"
            title="Each material referenced on this plate routes to one slot on the bound printer."
          >
            <span className="sp-config-divider-label">Materials</span>
          </div>
          <div className="sp-config-row sp-config-materials">
            {materials.map((mat) => (
              <MaterialChip
                key={`mat-${mat}`}
                material={mat}
                current={slotFor(mat)}
                slots={slots}
                filamentByIdentity={filamentByIdentity}
                useCount={useCountByMaterial.get(mat) ?? 0}
                onPickSlot={(slot) => onPickMaterialSlot(mat, slot)}
                onClear={() => onClearMaterialSlot(mat)}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}
