// Pre-send dialog — a final chance to allocate each model material to a
// physical slot/toolhead (Bambu Studio-style), plus the per-print toggles
// (bed leveling, flow / vibration calibration, timelapse). Shown when Send
// is clicked on a printer whose driver supports them (Bambu, Snapmaker U1).
//
// Material→slot picks write straight into the plate's `material_to_slot`
// binding (`project_set_material_slot` — the SAME storage the settings
// panel's binding uses), so an edit here is a real, persisted rebind. It's
// pure print-time routing: the firmware maps at print start (MAP_TABLE /
// ams_mapping), so a same-type rebind doesn't re-slice. To keep the
// already-sliced G-code valid, the slot picker only offers slots holding a
// *compatible filament type* (matching Bambu Studio, which greys out
// mismatched trays) — so a pick can never change the baked filament.
//
// The toggles are sticky per printer instance: the dialog seeds from the
// instance's persisted `send_options` and persists them on Send (not on
// Cancel). Labels track the vendor's own terminology (Bambu "vibration
// calibration" ⇔ U1 "input shaper calibration").

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ModalBackdrop } from "../ui/Modal";
import type {
  FlatSlotOption,
  PrinterInstance,
  SendOptions,
  SlotRef,
} from "../printer/printerInstance";
import type { PlateSnapshot } from "../viewport/types";
import { MaterialChip } from "../material/MaterialChip";
import { boundMaterials, slotForMaterial } from "../material/materials";
import { useFilamentCatalog } from "../material/useFilamentCatalog";

export interface SendOptionsDialogProps {
  /** Driver kind — picks the vendor-specific option labels. The caller
   *  only opens this dialog for kinds that support send options. */
  kind: "bambu" | "u1";
  /** The instance's persisted options, seeding the checkboxes. */
  initial: SendOptions;
  /** Active plate — supplies the materials + their current routing. */
  plate: PlateSnapshot | null;
  /** Bound printer instance — supplies the target slots + their filaments. */
  instance: PrinterInstance | null;
  /** Print + persist with the (possibly edited) options. */
  onSend: (options: SendOptions) => void;
  onCancel: () => void;
}

async function setMaterialSlot(
  plateId: number,
  modelMaterial: number,
  slot: SlotRef,
): Promise<void> {
  await invoke("project_set_material_slot", { plateId, modelMaterial, slot });
}

async function clearMaterialSlot(
  plateId: number,
  modelMaterial: number,
): Promise<void> {
  await invoke("project_clear_material_slot", { plateId, modelMaterial });
}

/** One labeled checkbox row. */
function OptionRow({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}): React.JSX.Element {
  return (
    <label className="send-opt-row" title={hint}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

export function SendOptionsDialog({
  kind,
  initial,
  plate,
  instance,
  onSend,
  onCancel,
}: SendOptionsDialogProps): React.JSX.Element {
  const [options, setOptions] = useState<SendOptions>(initial);
  const set = (patch: Partial<SendOptions>): void =>
    setOptions((prev) => ({ ...prev, ...patch }));

  const { byIdentity: filamentByIdentity } = useFilamentCatalog();
  const slots: FlatSlotOption[] = instance?.slots ?? [];
  const materials = boundMaterials(plate);

  // Object count per material, for the ×N badge.
  const useCountByMaterial = new Map<number, number>();
  for (const obj of plate?.objects ?? []) {
    const m = obj.extruder_id ?? 1;
    useCountByMaterial.set(m, (useCountByMaterial.get(m) ?? 0) + 1);
  }

  const baseTypeOf = (slot: FlatSlotOption | null): string | null =>
    slot?.filament_identity
      ? (filamentByIdentity.get(slot.filament_identity)?.base_type ?? null)
      : null;

  const showMaterials = plate != null && instance != null && materials.length > 0;

  return (
    <ModalBackdrop
      onDismiss={onCancel}
      cardClassName="psm-discard-card send-options-card"
      ariaLabelledBy="send-options-title"
    >
      <h3 id="send-options-title" className="psm-discard-title">
        Send to printer
      </h3>
      {showMaterials && (
        <>
          <div className="send-opt-section-label">Materials</div>
          <div className="send-opt-materials">
            {materials.map((mat) => {
              const current = slotForMaterial(
                mat,
                plate.material_to_slot ?? {},
                slots,
              );
              // Constrain routing to slots holding a compatible filament
              // type, so the pick never invalidates the sliced G-code. An
              // unmapped material (no current type) has no constraint.
              const currentType = baseTypeOf(current);
              return (
                <MaterialChip
                  key={`mat-${mat}`}
                  material={mat}
                  current={current}
                  slots={slots}
                  filamentByIdentity={filamentByIdentity}
                  useCount={useCountByMaterial.get(mat) ?? 0}
                  isSlotEnabled={(s) =>
                    currentType == null || baseTypeOf(s) === currentType
                  }
                  onPickSlot={(slot) =>
                    void setMaterialSlot(plate.plate_id, mat, slot).catch((e) =>
                      console.error("[send-dialog] setMaterialSlot failed", e),
                    )
                  }
                  onClear={() =>
                    void clearMaterialSlot(plate.plate_id, mat).catch((e) =>
                      console.error("[send-dialog] clearMaterialSlot failed", e),
                    )
                  }
                />
              );
            })}
          </div>
        </>
      )}
      <div className="send-opt-section-label">Options</div>
      <div className="send-opt-list">
        <OptionRow
          label="Auto bed leveling"
          hint="Check the heatbed's flatness before printing."
          checked={options.bed_leveling}
          onChange={(v) => set({ bed_leveling: v })}
        />
        <OptionRow
          label={
            kind === "bambu"
              ? "Flow dynamics calibration"
              : "Flow calibration"
          }
          hint="Calibrate dynamic flow / pressure advance before printing."
          checked={options.flow_calibration}
          onChange={(v) => set({ flow_calibration: v })}
        />
        <OptionRow
          label={
            kind === "bambu"
              ? "Vibration calibration"
              : "Input shaper calibration"
          }
          hint="Calibrate vibration compensation before printing."
          checked={options.vibration_calibration}
          onChange={(v) => set({ vibration_calibration: v })}
        />
        <OptionRow
          label="Timelapse"
          hint="Record a timelapse of the print with the built-in camera."
          checked={options.timelapse}
          onChange={(v) => set({ timelapse: v })}
        />
      </div>
      <div className="send-opt-actions">
        <button type="button" className="tb-btn" onClick={onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="tb-btn primary"
          onClick={() => onSend(options)}
        >
          Send
        </button>
      </div>
    </ModalBackdrop>
  );
}
