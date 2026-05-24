// Typed Tauri invoke wrappers for the driver layer (PR-7a-7).
//
// One named function per backend command; thin enough to mock in
// tests with a single `vi.mock("./invokes")`. Argument names use
// camelCase — Tauri auto-translates to the snake_case the Rust
// commands expect.

import { invoke } from "@tauri-apps/api/core";
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
): Promise<SendHandle> {
  return invoke<SendHandle>("driver_send_plate", {
    id,
    plateId,
    gcodePath,
  });
}

/** Dry-run variant of `driverSendPlate`: bundle is wrapped, then
 * neutered (E values stripped, heaters commented out) before
 * upload. Printer exercises every XY motion with zero filament
 * flow. */
export function driverDrySendPlate(
  id: DriverId,
  plateId: number,
  gcodePath: string,
): Promise<SendHandle> {
  return invoke<SendHandle>("driver_dry_send_plate", {
    id,
    plateId,
    gcodePath,
  });
}
