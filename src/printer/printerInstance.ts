// Mirror of the Rust `PrinterInstance` shape.
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
 *  set — Bambu carries an 8-hex-char LAN access code; U1 and generic
 *  Moonraker printers carry a Moonraker port (usually 80). Device
 *  serial is NOT stored here: the Bambu driver probes it at connect
 *  time and Moonraker printers have none, so it never enters the
 *  persisted connection. */
export type ConnectionInfo =
  | { kind: "bambu"; host: string; access_code: string }
  | { kind: "moonraker"; host: string; port: number }
  | { kind: "u1"; host: string; port: number };

/** Sticky per-print toggles, mirroring Rust's `SendOptions`. One shape
 *  serves both vendors (U1 "shaper calibrate" ⇔ Bambu "vibration
 *  calibration"); the send dialog labels per kind. */
export interface SendOptions {
  bed_leveling: boolean;
  flow_calibration: boolean;
  vibration_calibration: boolean;
  timelapse: boolean;
}

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
  /** Sticky per-print send options; edited by the send dialog. */
  send_options: SendOptions;
  /** Pre-labeled, flattened slots — the single source of truth for slot
   *  display, computed Rust-side from topology (`PrinterInstanceView`).
   *  Renderers read these; they don't re-derive labels. */
  slots: FlatSlotOption[];
  /** Installed AMS-unit count, derived backend-side from slot topology. */
  ams_units: number;
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

/** Replace the instance's sticky per-print send options. Persisted in
 *  the user library; emits `printer:instance_changed`. */
export async function setInstanceSendOptions(
  id: string,
  options: SendOptions,
): Promise<PrinterInstance> {
  return invoke<PrinterInstance>("printer_instance_set_send_options", {
    id,
    options,
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

/** A flattened, pre-labeled slot — mirrors Rust's `SlotView`. Both the
 *  long `label` and the compact `short_label` are computed backend-side
 *  from topology (extruder count, per-slot `feed`); the renderer reads
 *  them rather than re-deriving. `ref` locates the slot for write-back. */
export interface FlatSlotOption {
  ref: SlotRef;
  label: string;
  /** Compact chip label ("A:1", "1", "Ext", "T1"). */
  short_label: string;
  feed: FeedKind;
  filament_identity: string | null;
  color: string | null;
  /** RFID tag id of the synced spool, mirrored from `SlotBinding`.
   *  Drives the read-only gate on RFID-auto-detected slots. */
  tag_uid: string | null;
}
