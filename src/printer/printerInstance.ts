// Mirror of the Rust `PrinterInstance` shape (PR-S-3 + PR-S-7).
//
// Used by the slot-binding panel to render the per-extruder,
// per-slot topology of the active plate's bound printer. Writes
// go back through `printer_instance_set_slot_filament`; the
// backend emits `printer:instance_changed` so consumers refetch.

import { invoke } from "@tauri-apps/api/core";

/** Per-slot feed kind — drives the per-extruder feed-mix gate.
 *  Mirrors `core::printer::instance::FeedKind`. */
export type FeedKind = "direct" | "ams";

/** Pointer into a PrinterInstance's `(extruder, slot)` grid.
 *  0-based indices. */
export interface SlotRef {
  extruder: number;
  slot: number;
}

export interface SlotBinding {
  /** Display label — "Direct" / "Ext" / "AMS:1" / "" (empty when
   *  the extruder label is enough). */
  label: string;
  feed: FeedKind;
  /** `null` when no filament is loaded in this slot. */
  filament_identity: string | null;
  /** Spool color as a CSS hex string ("#ff8800"). `null` means
   *  unassigned — the picker shows a neutral placeholder. */
  color: string | null;
}

export interface NozzleSku {
  diameter_mm: number;
  material:
    | "brass"
    | "hardened"
    | "stainless"
    | "high_flow_hardened"
    | "high_flow_stainless";
}

export interface ExtruderState {
  /** Display label — "T0" / "T1" / "" (empty when the extruder is
   *  solo and the slot's label carries the full identity). */
  label: string;
  installed_nozzle: NozzleSku;
  slots: SlotBinding[];
}

export interface BedRef {
  identity: string;
}

export interface ConnectionInfo {
  host: string;
  serial: string;
  access_code: string;
  dev_mode: boolean;
}

export interface PrinterInstance {
  id: string;
  display_name: string;
  vendor_profile_ref: string;
  printer_fragment_slug: string;
  default_filament_fragment_slug: string;
  default_process_fragment_slug: string;
  connection: ConnectionInfo | null;
  extruders: ExtruderState[];
  bed: BedRef;
  config_overrides: Record<string, string>;
}

/** Bundled (and future user-library) instances the picker offers. */
export async function listPrinterInstances(): Promise<PrinterInstance[]> {
  return invoke<PrinterInstance[]>("printer_instance_list");
}

/** Snapshot a single instance by id. Returns `null` when the id
 *  isn't registered. */
export async function getPrinterInstance(
  id: string,
): Promise<PrinterInstance | null> {
  return invoke<PrinterInstance | null>("printer_instance_get", { id });
}

/** Bind (or clear, with `null`) the filament loaded in a slot.
 *  Returns the post-mutation instance snapshot so callers can
 *  re-render without a second round-trip. Backend also emits
 *  `printer:instance_changed`. */
export async function setSlotFilament(
  id: string,
  extruderIdx: number,
  slotIdx: number,
  filamentIdentity: string | null,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_slot_filament", {
    id,
    extruderIdx,
    slotIdx,
    filamentIdentity,
  });
}

/** Set (or clear, with `null`) a slot's user-assigned spool color.
 *  Hex string ("#ff8800"). Same event-emission contract as
 *  `setSlotFilament`. A future driver-side AMS sync writes the
 *  same field. */
export async function setSlotColor(
  id: string,
  extruderIdx: number,
  slotIdx: number,
  color: string | null,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_slot_color", {
    id,
    extruderIdx,
    slotIdx,
    color,
  });
}

/** Change the diameter of the nozzle currently installed on the
 *  named extruder. Material is preserved — the picker only writes
 *  diameter swaps in the MVP. Emits `printer:instance_changed`. */
export async function setExtruderNozzleDiameter(
  id: string,
  extruderIdx: number,
  diameterMm: number,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>(
    "printer_instance_set_extruder_nozzle_diameter",
    { id, extruderIdx, diameterMm },
  );
}

/** Change the bed currently loaded on this instance. Validated
 *  backend-side against the bound printer profile's
 *  `supported_build_plates`. Emits `printer:instance_changed`. */
export async function setInstanceBed(
  id: string,
  bedIdentity: string,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_bed", {
    id,
    bedIdentity,
  });
}

/** Register a fresh `PrinterInstance` from a bundled printer
 *  identity + display name + AMS unit count. Returns the new
 *  instance (UUID-keyed) so the caller can immediately rebind it
 *  to a plate. Emits `printer:instance_changed`. */
export async function createInstance(
  printerIdentity: string,
  displayName: string,
  amsUnits: number,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_create", {
    printerIdentity,
    displayName,
    amsUnits,
  });
}

/** Remove a registered `PrinterInstance`. Plates bound to this
 *  instance become dangling; the slice gate refuses to run on
 *  them and the picker surfaces them as "unbound." */
export async function deleteInstance(id: string): Promise<void> {
  return invoke("printer_instance_delete", { id });
}

/** Compose a flat list of slot picker options across the instance's
 *  extruder × slot grid. The label combines extruder + slot labels
 *  with " — " when both are non-empty; one label alone otherwise.
 *  Includes the SlotRef so the picker can write back. */
export interface FlatSlotOption {
  ref: SlotRef;
  label: string;
  feed: FeedKind;
  filament_identity: string | null;
  color: string | null;
}

export function flattenSlots(instance: PrinterInstance): FlatSlotOption[] {
  const out: FlatSlotOption[] = [];
  instance.extruders.forEach((ext, eIdx) => {
    ext.slots.forEach((slot, sIdx) => {
      const parts: string[] = [];
      if (ext.label) parts.push(ext.label);
      if (slot.label) parts.push(slot.label);
      const label = parts.length > 0 ? parts.join(" — ") : `Slot ${sIdx + 1}`;
      out.push({
        ref: { extruder: eIdx, slot: sIdx },
        label,
        feed: slot.feed,
        filament_identity: slot.filament_identity,
        color: slot.color,
      });
    });
  });
  return out;
}

/** Test whether picking `candidate` for one material while
 *  `already_used` slots are pinned elsewhere would create a Bambu
 *  feed-mix conflict (Direct + Ams on the same extruder). The picker
 *  uses this to grey out options. */
export function isFeedMixConflict(
  candidate: FlatSlotOption,
  alreadyUsed: ReadonlyArray<FlatSlotOption>,
): boolean {
  for (const used of alreadyUsed) {
    if (
      used.ref.extruder === candidate.ref.extruder &&
      used.feed !== candidate.feed
    ) {
      return true;
    }
  }
  return false;
}
