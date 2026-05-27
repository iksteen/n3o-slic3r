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
  setSlotColor,
  setSlotFilament,
  type FlatSlotOption,
  type PrinterInstance,
  type SlotRef,
} from "../printer/printerInstance";
import { getDriverId } from "../driver/credentialsCache";
import { MaterialChip } from "./MaterialChip";
import { SlotChipStrip } from "./SlotChipStrip";
import type { FilamentSummary } from "./filamentSummary";

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

  const filamentByIdentity = useMemo(() => {
    const map = new Map<string, FilamentSummary>();
    for (const f of filaments) map.set(f.identity, f);
    return map;
  }, [filaments]);

  if (!plateId || !plate || !instance) {
    return null;
  }

  const onApplyPick = (
    slot: SlotRef,
    pick: { identity: string; color: string },
  ): void => {
    if (!instance) return;
    // Two backend writes — the second runs after the first resolves
    // so a fast-emitted `printer:instance_changed` between them
    // doesn't show a half-updated state. Either failing is logged
    // but we still try the other.
    void setSlotFilament(instance.id, slot.extruder, slot.slot, pick.identity)
      .catch((err) =>
        console.error("[slot-binding] setSlotFilament failed", err),
      )
      .then(() =>
        setSlotColor(instance.id, slot.extruder, slot.slot, pick.color).catch(
          (err) => console.error("[slot-binding] setSlotColor failed", err),
        ),
      );
  };

  // Manual sync — pulls the printer's current spool loadout into
  // the instance via the live driver (PR-7c-2). Resolves a
  // DriverId from the credentials cache (same key the header
  // device controls use). Rejects when no driver is registered or
  // when the backend command errors; SyncSlotsLabel turns that
  // into the error-triangle state.
  const onSyncSlots = async (): Promise<void> => {
    if (!instance) throw new Error("no instance");
    const driverId = getDriverId(instance.vendor_profile_ref);
    if (driverId == null) {
      throw new Error("printer not connected");
    }
    await invoke("printer_instance_sync_from_driver", {
      instanceId: instance.id,
      driverId,
    });
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
                current={slotForMaterial(mat)}
                slots={slots}
                totalExtruders={instance.extruders.length}
                extruderSlots={instance.extruders.map((e) => e.slots)}
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
