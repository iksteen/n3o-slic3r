// Typed Tauri invoke wrappers for the driver layer (PR-7a-7).
//
// One named function per backend command; thin enough to mock in
// tests with a single `vi.mock("./invokes")`. Argument names use
// camelCase — Tauri auto-translates to the snake_case the Rust
// commands expect.

import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  DriverConfig,
  DriverId,
  DriverSummary,
  PrinterCommand,
  PrinterStatus,
  SendHandle,
} from "./types";

/** Register a fresh driver instance. Returns the assigned id.
 * Also spawns the backend status-bridge task so subsequent
 * `driver:status_update` events fire for this driver. */
export function driverRegister(config: DriverConfig): Promise<DriverId> {
  return invoke<DriverId>("driver_register", { config });
}

/** Test a connection config without registering a driver. Resolves
 *  when the transient connection reaches `Connected`; rejects with the
 *  printer's reason (or a timeout message) otherwise. Nothing is
 *  persisted and the reconciler is not touched. */
export function driverTestConnection(config: DriverConfig): Promise<void> {
  return invoke<void>("driver_test_connection", { config });
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

/** Cheap snapshot of every registered driver. */
export function driverList(): Promise<DriverSummary[]> {
  return invoke<DriverSummary[]>("driver_list");
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

/** Send the plate's last-sliced raw G-code (at `gcodePath`) to
 * the driver as a `.gcode.3mf` bundle. Backend wraps the raw
 * gcode via PR-3-10's writer; PR-7c-7 will replace the stub
 * with full sync-on-send + AMS metadata. */
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
