// Typed Tauri invoke wrappers for the driver layer.
//
// One named function per backend command; thin enough to mock in
// tests with a single `vi.mock("./invokes")`. Argument names use
// camelCase — Tauri auto-translates to the snake_case the Rust
// commands expect.

import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  DriverConfig,
  DriverId,
  PrinterCommand,
  PrinterStatus,
  SendHandle,
} from "./types";

/** Register a fresh driver instance. Returns the assigned id.
 * Also spawns the backend status-bridge task so subsequent
 * `driver:status_update` events fire for this driver. `instanceId` keys
 * the U1 pairing token, so a paired printer's status rides the mTLS MQTT
 * bus (remote-capable) instead of the LAN WebSocket. */
export function driverRegister(
  config: DriverConfig,
  instanceId: string,
): Promise<DriverId> {
  return invoke<DriverId>("driver_register", { config, instanceId });
}

/** Test a connection config without registering a driver. Resolves
 *  when the transient connection reaches `Connected`; rejects with the
 *  printer's reason (or a timeout message) otherwise. Nothing is
 *  persisted and the reconciler is not touched. */
export function driverTestConnection(
  config: DriverConfig,
  instanceId: string,
): Promise<void> {
  return invoke<void>("driver_test_connection", { config, instanceId });
}

/** Disconnect + remove a driver from the registry. */
export function driverUnregister(id: DriverId): Promise<void> {
  return invoke<void>("driver_unregister", { id });
}

/** Open the driver's connection (rumqttc loop for Bambu). */
export function driverConnect(id: DriverId): Promise<void> {
  return invoke<void>("driver_connect", { id });
}

/** Close the driver's connection cleanly. */
export function driverDisconnect(id: DriverId): Promise<void> {
  return invoke<void>("driver_disconnect", { id });
}

/** Latest cached status snapshot — reads the driver's internal
 * watch channel without contacting the printer. Useful for the
 * `useDriverStatus` initial-fetch pass. */
export function driverStatus(id: DriverId): Promise<PrinterStatus> {
  return invoke<PrinterStatus>("driver_status", { id });
}

/** Pause / resume / stop the current print. */
export function driverCommand(
  id: DriverId,
  cmd: PrinterCommand,
): Promise<void> {
  return invoke<void>("driver_command", { id, cmd });
}

/** Run a pressure-advance calibration for a slot's filament and store the
 * measured K (keyed by filament identity × color × nozzle) so future slices
 * use it over the profile default. Long-running (heats + sweeps — minutes);
 * resolves to the measured K. */
export function driverCalibratePa(
  driverId: DriverId,
  instanceId: string,
  extruderIdx: number,
  slotIdx: number,
): Promise<number> {
  return invoke<number>("driver_calibrate_pa", {
    driverId,
    instanceId,
    extruderIdx,
    slotIdx,
  });
}

/** Park the active toolhead back in its dock. Called once after a PA
 * calibration cycle so the printer isn't left holding the last picked
 * toolhead. No-op on non-toolchanger printers. */
export function driverParkExtruder(driverId: DriverId): Promise<void> {
  return invoke<void>("driver_park_extruder", { driverId });
}

/** One measured K from a Bambu batched calibration — mirrors Rust's
 * `BambuCaliSlotK`. `confidence`: 0 = success, 1 = uncertain, 2 = failed. */
export interface BambuCaliSlotK {
  extruder_index: number;
  slot_index: number;
  k_value: number;
  confidence: number;
}

/** Run Bambu Flow-Dynamics calibration for a batch of slots in ONE printer job
 * (unlike the U1's per-toolhead loop), storing each measured K color-keyed and
 * returning it per slot. `slots` is a list of `[extruderIdx, slotIdx]`. */
export function driverCalibratePaBatch(
  driverId: DriverId,
  instanceId: string,
  slots: [number, number][],
): Promise<BambuCaliSlotK[]> {
  return invoke<BambuCaliSlotK[]>("driver_calibrate_pa_bambu", {
    driverId,
    instanceId,
    slots,
  });
}

/** Send the plate's last-sliced raw G-code (at `gcodePath`) to
 * the driver as a `.gcode.3mf` bundle. Backend wraps the raw
 * gcode into the bundle. */
export function driverSendPlate(
  id: DriverId,
  plateId: number,
  gcodePath: string,
  thumbnailPngBase64?: string | null,
): Promise<SendHandle> {
  return invoke<SendHandle>("driver_send_plate", {
    id,
    plateId,
    gcodePath,
    thumbnailPngBase64: thumbnailPngBase64 ?? null,
  });
}

/** Human-readable message from a rejected DriverError. serde serializes the
 *  enum externally-tagged: unit variants (e.g. "Cancelled") arrive as a bare
 *  string, the rest as `{ Variant: "message" }`. Callers branch on the variant
 *  before formatting (e.g. `e === "Cancelled"`); this is the display fallback. */
export function driverErrorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e !== null && typeof e === "object") {
    const v = Object.values(e as Record<string, unknown>)[0];
    if (typeof v === "string") return v;
  }
  return String(e);
}

/** Cancel an in-flight send to `id`. No-op if nothing is uploading; the
 *  pending `driverSendPlate` rejects with a cancellation error (which the
 *  caller treats as a user action, not a failure). */
export function driverSendCancel(id: DriverId): Promise<void> {
  return invoke<void>("driver_send_cancel", { id });
}

/** Open a live camera stream for a printer instance. The backend pushes
 *  raw JPEG frames over `channel` (each delivered as an `ArrayBuffer`).
 *  Lifecycle is frontend-driven: call this when the camera panel becomes
 *  active and `cameraStop` when it's hidden. Rejects for backends without
 *  camera support (only Bambu LAN cameras are wired today). */
export function cameraStart(
  instanceId: string,
  config: DriverConfig,
  channel: Channel<ArrayBuffer>,
): Promise<void> {
  return invoke<void>("camera_start", { instanceId, config, channel });
}

/** Close the live camera stream for a printer instance. Idempotent. */
export function cameraStop(instanceId: string): Promise<void> {
  return invoke<void>("camera_stop", { instanceId });
}

/** U1 LAN pairing state — whether the instance holds mTLS material, and
 *  the paired printer serial (for display). Never carries key material. */
export interface PairingStatus {
  paired: boolean;
  serial: string | null;
}

/** Run the Snapmaker U1 pairing dance against `host`. Resolves once the
 *  user taps Approve on the printer (or rejects on the ~60s timeout). The
 *  mTLS keypair is persisted server-side; only the status comes back. */
export function u1Pair(instanceId: string, host: string): Promise<PairingStatus> {
  return invoke<PairingStatus>("u1_pair", { instanceId, host });
}

/** Whether the U1 instance is paired (and its serial). */
export function u1PairingStatus(instanceId: string): Promise<PairingStatus> {
  return invoke<PairingStatus>("u1_pairing_status", { instanceId });
}

/** Forget the U1 instance's pairing. Idempotent. */
export function u1Unpair(instanceId: string): Promise<void> {
  return invoke<void>("u1_unpair", { instanceId });
}

/** Diagnostic: wrap the plate's gcode into the same `.gcode.3mf`
 * bundle the send path produces and write it to disk. Lets us
 * grab exactly what we'd send for offline diffing against BBS /
 * other slicer outputs. No driver / network involvement. */
export function driverExportPlate(
  plateId: number,
  gcodePath: string,
  outputPath: string,
  thumbnailPngBase64?: string | null,
): Promise<void> {
  return invoke<void>("driver_export_plate", {
    plateId,
    gcodePath,
    outputPath,
    thumbnailPngBase64: thumbnailPngBase64 ?? null,
  });
}
