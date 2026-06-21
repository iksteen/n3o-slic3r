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
  feed: FeedKind;
  /** `null` when no filament is loaded in this slot. */
  filament_identity: string | null;
  /** Spool color as a CSS hex string ("#ff8800"). `null` means
   *  unassigned — the picker shows a neutral placeholder. */
  color: string | null;
  /** Last-synced RFID tag id (Bambu `tag_uid`), or `null`/all-zeros
   *  when the spool wasn't auto-detected via RFID. When set (see
   *  `isRfidDetected`), the slot is printer-authoritative and the UI
   *  blocks editing it. */
  tag_uid: string | null;
}

export interface NozzleSku {
  /** Nozzle diameter as a **string symbol** ("0.4", "0.25",
   *  "0.4+0.6") — never a number. The cascade composer matches
   *  it to a `nozzles/<diameter>.toml` filename by exact-string
   *  lookup, and the Quality picker filters fragments by exact-set
   *  membership; arithmetic on it would be a category error. */
  diameter: string;
  material:
    | "brass"
    | "hardened"
    | "stainless"
    | "high_flow_hardened"
    | "high_flow_stainless";
}

export interface ExtruderState {
  installed_nozzle: NozzleSku;
  slots: SlotBinding[];
}

export interface BedRef {
  identity: string;
}

/** Per-driver connection settings, mirroring Rust's tagged-enum
 *  `ConnectionInfo`. The `kind` discriminator switches the field
 *  set — Bambu carries an 8-digit LAN access code; U1 carries a
 *  Moonraker port (usually 80). Device serial is NOT stored here: the
 *  drivers probe it at connect time, so it's a runtime-only concern
 *  on `DriverConfig`, not part of the persisted connection. */
export type ConnectionInfo =
  | { kind: "bambu"; host: string; access_code: string }
  | { kind: "u1"; host: string; port: number };

export interface PrinterInstance {
  id: string;
  display_name: string;
  vendor_profile_ref: string;
  printer_fragment_slug: string;
  default_filament_fragment_slug: string;
  quality_profile: string;
  connection: ConnectionInfo | null;
  extruders: ExtruderState[];
  bed: BedRef;
  config_overrides: Record<string, string>;
}

/** Bundled (and future user-library) instances the picker offers. */
export async function listPrinterInstances(): Promise<PrinterInstance[]> {
  return invoke<PrinterInstance[]>("printer_instance_list");
}

/** Derive the AMS-unit count from an instance's slot topology.
 *  Counts AMS-feed slots on the first extruder and divides by 4
 *  (every AMS unit ships exactly 4 slots; the trailing slot is
 *  always direct-fed). Matches the backend's `create_instance` /
 *  `set_instance_ams_units` formula: `ams_units * 4 + 1` total
 *  slots, AMS-first ordering. */
export function amsUnitsOf(instance: PrinterInstance): number {
  const slots = instance.extruders[0]?.slots ?? [];
  const amsSlots = slots.filter((s) => s.feed === "ams").length;
  return Math.floor(amsSlots / 4);
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
 *  named extruder. `diameter` is a string symbol ("0.4", "0.25")
 *  — see [NozzleSku.diameter] for why. Material is preserved.
 *  Emits `printer:instance_changed`. */
export async function setExtruderNozzleDiameter(
  id: string,
  extruderIdx: number,
  diameter: string,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>(
    "printer_instance_set_extruder_nozzle_diameter",
    { id, extruderIdx, diameter },
  );
}

/** Set (or clear, with `value = null`) a machine-settings override on
 *  the instance — a Printer-bucket key in `config_overrides`. The backend
 *  rejects non-Printer-bucket keys. Emits `printer:instance_changed`. */
export async function setMachineOverride(
  id: string,
  key: string,
  value: string | null,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_config_override", {
    id,
    key,
    value,
  });
}

/** Resolve the instance's cascade to a flat `key → value` map — the
 *  machine panel shows these as each option's base (pre-override) value. */
export async function resolvedInstanceConfig(
  id: string,
): Promise<Record<string, string>> {
  return invoke<Record<string, string>>("printer_instance_resolved_config", {
    id,
  });
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

/** Atomic delete + rebind: hand the backend the plate IDs to
 *  rebind (with the fallback id) or unbind (when fallback is
 *  null), then the instance to delete. One registry+project
 *  lock; either everything commits or nothing does. The frontend
 *  no longer has a partial-commit window between a sequential
 *  rebind loop and the delete itself. */
export async function deleteInstanceWithReassign(
  id: string,
  fallbackInstanceId: string | null,
  plateIds: number[],
): Promise<void> {
  return invoke("printer_instance_delete_with_reassign", {
    id,
    fallbackInstanceId,
    plateIds,
  });
}

/** Rename an instance. Trims whitespace and rejects empty
 *  (backend-side; the modal validates locally too). Emits
 *  `printer:instance_changed`. */
export async function setInstanceDisplayName(
  id: string,
  displayName: string,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_display_name", {
    id,
    displayName,
  });
}

/** Change the AMS-unit count on an AMS-style printer. Re-derives the
 *  slot topology (`(amsUnits * 4 + 1)`); preserved bindings stay
 *  positional, dropped bindings warn server-side. Rejects for
 *  toolchangers and for values above `profile.ams_max`. Emits
 *  `printer:instance_changed`. */
export async function setInstanceAmsUnits(
  id: string,
  amsUnits: number,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_ams_units", {
    id,
    amsUnits,
  });
}

/** Write (or clear, with `null`) the instance's network connection.
 *  The on-disk user library persists this so the same physical
 *  printer's connection survives across app restarts. Emits
 *  `printer:instance_changed`. */
export async function setInstanceConnection(
  id: string,
  connection: ConnectionInfo | null,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_connection", {
    id,
    connection,
  });
}

/** Atomic multi-field update — applied under one registry lock,
 *  with one persist + one printer:instance_changed emit. Omit a
 *  field (or leave it undefined) to leave it unchanged. Setting
 *  To CLEAR a connection use `clearConnection: true` — NOT
 *  `connection: null`. The backend treats a null/omitted
 *  `connection` as "leave unchanged" (serde collapses it to
 *  `None`), so only `clearConnection` actually clears; the type
 *  below omits `| null` to keep that the single clear path. The
 *  settings modal calls this once on Save instead of issuing three
 *  per-field IPCs in sequence — closing the partial-success window
 *  where one mutator landed and a later one threw. */
export interface InstancePatch {
  displayName?: string;
  amsUnits?: number;
  connection?: ConnectionInfo;
  clearConnection?: boolean;
}
export async function updateInstance(
  id: string,
  patch: InstancePatch,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_update", {
    id,
    patch: {
      display_name: patch.displayName,
      ams_units: patch.amsUnits,
      connection: patch.connection,
      clear_connection: patch.clearConnection ?? false,
    },
  });
}

/** Compose a flat list of slot picker options across the instance's
 *  extruder × slot grid. Labels are derived from structure (extruder
 *  count, slot count, per-slot `feed`); they're not stored on the
 *  instance so a runtime topology change (user attaches a second
 *  AMS unit, swaps a nozzle) doesn't risk stale labels lingering
 *  in memory. Includes the SlotRef so the picker can write back. */
export interface FlatSlotOption {
  ref: SlotRef;
  label: string;
  feed: FeedKind;
  filament_identity: string | null;
  color: string | null;
  /** RFID tag id of the synced spool, mirrored from `SlotBinding`.
   *  Drives the read-only gate on RFID-auto-detected slots. */
  tag_uid: string | null;
}

/** Display label for an extruder, given its 0-based position and the
 *  total number of extruders on the printer. Multi-extruder
 *  printers (toolchangers) get 1-based `T1..TN`; single-extruder
 *  printers get an empty label (the slot label carries identity). */
export function deriveExtruderLabel(
  extIdx: number,
  totalExtruders: number,
): string {
  if (totalExtruders <= 1) return "";
  return `T${extIdx + 1}`;
}

/** Position of a slot within an AMS-style multi-slot extruder, shared
 *  by the two slot-label formatters. Ams-feed slots are numbered
 *  1-based across the extruder, grouped into AMS-unit letters of 4 once
 *  there are >4 (multi-AMS-unit topology); the trailing Direct-feed slot
 *  is the external spool. */
type SlotPosition =
  | { kind: "ams"; unitIdx: number; idxInUnit: number; multiUnit: boolean }
  | { kind: "direct" }
  | { kind: "none" };

function amsSlotPosition(
  slotIdx: number,
  slots: readonly SlotBinding[],
): SlotPosition {
  const multiUnit = slots.filter((s) => s.feed === "ams").length > 4;
  let idxInUnit = 0;
  let unitIdx = 0;
  for (let i = 0; i < slots.length; i++) {
    if (slots[i].feed === "ams") {
      if (idxInUnit === 4) {
        idxInUnit = 0;
        unitIdx += 1;
      }
      idxInUnit += 1;
      if (i === slotIdx) return { kind: "ams", unitIdx, idxInUnit, multiUnit };
    } else if (i === slotIdx) {
      return { kind: "direct" };
    }
  }
  return { kind: "none" };
}

/** Long-form label for a slot, used in tooltips + the picker
 *  dropdown. Slot-scope only (doesn't include the extruder label) —
 *  `flattenSlots` combines this with the extruder label via " — ".
 *
 *  Single-slot extruders: surface identity through the extruder
 *  label on multi-extruder printers, through a `Direct` / `AMS:1`
 *  feed-kind label on single-extruder printers.
 *  Multi-slot extruders are AMS-style (`AMS A:1..AMS B:4`, trailing
 *  `Ext`) — see [`amsSlotPosition`]. */
function deriveSlotLabel(
  slotIdx: number,
  slots: readonly SlotBinding[],
  totalExtruders: number,
): string {
  if (slots.length === 1) {
    if (totalExtruders > 1) return "";
    return slots[0].feed === "direct" ? "Direct" : "AMS:1";
  }
  const pos = amsSlotPosition(slotIdx, slots);
  if (pos.kind === "ams") {
    return pos.multiUnit
      ? `AMS ${String.fromCharCode(65 + pos.unitIdx)}:${pos.idxInUnit}`
      : `AMS:${pos.idxInUnit}`;
  }
  return pos.kind === "direct" ? "Ext" : "";
}

/** Compact label for a slot pill — fits inside a 22px chip alongside
 *  a swatch + material tag. The row containing the chip already
 *  carries the noun ("Slots"), so each chip just needs its own
 *  position identifier.
 *
 *  Rules — slightly different from [`deriveSlotLabel`]'s long form:
 *  * Multi-extruder toolchanger (e.g. U1, XL): chip shows the
 *    extruder label (`T1`, `T2`, …). Each extruder has one Direct
 *    slot, so the extruder label is the slot identity.
 *  * Single-extruder + 1 slot (bambi without AMS): chip shows
 *    `Ext` (Direct) or `1` (AMS).
 *  * Single-extruder + multi-slot AMS:
 *    - One AMS unit: AMS slots are bare digits `1`, `2`, `3`, `4`.
 *      Trailing Direct slot is `Ext`.
 *    - Multiple AMS units: AMS slots get letter-prefixed `A:1`,
 *      `B:3` (no space, with colon). Trailing Direct slot is `Ext`.
 */
export function deriveSlotShortLabel(
  extIdx: number,
  totalExtruders: number,
  slotIdx: number,
  slots: readonly SlotBinding[],
): string {
  // Toolchanger: each extruder is one slot, the chip shows the
  // extruder identity (`T1`, `T2`, …).
  if (totalExtruders > 1) {
    return `T${extIdx + 1}`;
  }
  // Single-extruder + single slot — degenerate AMS-less printer.
  if (slots.length === 1) {
    return slots[0].feed === "direct" ? "Ext" : "1";
  }
  // Single-extruder + multi-slot AMS layout.
  const pos = amsSlotPosition(slotIdx, slots);
  if (pos.kind === "ams") {
    return pos.multiUnit
      ? `${String.fromCharCode(65 + pos.unitIdx)}:${pos.idxInUnit}`
      : `${pos.idxInUnit}`;
  }
  return pos.kind === "direct" ? "Ext" : "";
}

export function flattenSlots(instance: PrinterInstance): FlatSlotOption[] {
  const out: FlatSlotOption[] = [];
  const totalExt = instance.extruders.length;
  instance.extruders.forEach((ext, eIdx) => {
    const extLabel = deriveExtruderLabel(eIdx, totalExt);
    ext.slots.forEach((slot, sIdx) => {
      const slotLabel = deriveSlotLabel(sIdx, ext.slots, totalExt);
      const parts: string[] = [];
      if (extLabel) parts.push(extLabel);
      if (slotLabel) parts.push(slotLabel);
      const label = parts.length > 0 ? parts.join(" — ") : `Slot ${sIdx + 1}`;
      out.push({
        ref: { extruder: eIdx, slot: sIdx },
        label,
        feed: slot.feed,
        filament_identity: slot.filament_identity,
        color: slot.color,
        tag_uid: slot.tag_uid,
      });
    });
  });
  return out;
}

