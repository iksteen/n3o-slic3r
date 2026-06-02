// Shared material/slot helpers used by the Materials section
// (SlotBindingPanel), the Objects panel, and its MaterialPicker. A
// "material" is an object's 1-based `extruder_id`, routed to a printer
// slot via the plate's `material_to_slot` table; these centralize the
// lookup + display rules so the surfaces agree.

import type { CSSProperties } from "react";
import type { PlateSnapshot, SceneObject } from "../viewport/types";
import type { FlatSlotOption, SlotRef } from "../printer/printerInstance";

/** An object's material index — its `extruder_id`, defaulting to 1
 *  (unassigned inherits material 1). */
export function materialOf(obj: SceneObject): number {
  return obj.extruder_id ?? 1;
}

/** The 1-based material indices referenced by objects on the plate,
 *  sorted ascending. */
export function referencedMaterials(plate: PlateSnapshot | null): number[] {
  if (!plate) return [];
  const seen = new Set<number>();
  for (const obj of plate.objects) seen.add(materialOf(obj));
  return Array.from(seen).sort((a, b) => a - b);
}

/** Every material the plate actually uses: those referenced by objects
 *  PLUS those bound in `material_to_slot` without a corresponding object.
 *  A filament applied only via MMU face-painting is the latter case — the
 *  importer binds it as a plate material, but no object carries its
 *  `extruder_id` (the base object stays material 1; only some faces are the
 *  painted material). Mirrors the backend `Plate::material_count`, which
 *  also unions object extruders with `material_to_slot` keys. */
export function boundMaterials(plate: PlateSnapshot | null): number[] {
  if (!plate) return [];
  const seen = new Set<number>();
  for (const obj of plate.objects) seen.add(materialOf(obj));
  for (const key of Object.keys(plate.material_to_slot ?? {})) {
    const m = Number(key);
    if (Number.isInteger(m) && m > 0) seen.add(m);
  }
  return Array.from(seen).sort((a, b) => a - b);
}

/** The slot a material is routed to (via `material_to_slot`), resolved
 *  against the bound instance's flattened slots. */
export function slotForMaterial(
  material: number,
  materialToSlot: Record<number, SlotRef>,
  slots: FlatSlotOption[],
): FlatSlotOption | null {
  const pick = materialToSlot[material];
  if (!pick) return null;
  return (
    slots.find(
      (s) => s.ref.extruder === pick.extruder && s.ref.slot === pick.slot,
    ) ?? null
  );
}

/** A slot's display colour — only when a filament is actually loaded.
 *  A cached spool colour with no identity (e.g. the unloaded Bambu
 *  external feed, no RFID) reads as empty, not a solid swatch. */
export function slotColor(slot: FlatSlotOption | null): string | null {
  return slot?.filament_identity ? slot.color : null;
}

/** Inline style for a colour swatch — solid when loaded, a hollow
 *  dashed orb when empty. */
export function swatchStyle(color: string | null): CSSProperties {
  return {
    background: color ?? "transparent",
    border: color ? "none" : "1px dashed var(--text-muted)",
  };
}
