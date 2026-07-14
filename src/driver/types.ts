// Wire-shape mirrors of the Rust driver types.
//
// Sources of truth:
//   - `src-tauri/src/core/driver/status.rs`  → connection / job / temps / AMS
//   - `src-tauri/src/core/driver/traits.rs`  → DriverId / DriverConfig / SendHandle
//
// Drift between these typedefs and the Rust serde output is the
// most common cause of "silently works on one side" bugs in this
// codebase (the slice + scene mirrors taught us this). When the
// Rust types change, edit this file in the same commit.

/** `#[serde(transparent)] pub struct DriverId(pub u64)` — bare integer on the wire. */
export type DriverId = number;

/** Mirror of `DriverKind` (`#[serde(rename_all = "snake_case")]`) —
 *  lowercase on the wire. NOTE: the separate `DriverConfig` /
 *  `DriverExtra` enums below stay PascalCase-tagged; only the
 *  driver-kind discriminator is lowercase. */
export type DriverKind = "bambu" | "u1" | "moonraker";

/** Mirror of `DriverConfig` (`#[serde(tag="kind", content="data")]`). */
export type DriverConfig =
  | {
      kind: "Bambu";
      data: {
        host: string;
        access_code: string;
      };
    }
  | {
      kind: "U1";
      data: {
        host: string;
        port: number;
      };
    }
  | {
      /** Generic Klipper printer speaking vanilla Moonraker. No
       *  printer profile declares it yet — the U1 keeps its own kind
       *  for the vendor webcam stack. */
      kind: "Moonraker";
      data: {
        host: string;
        port: number;
      };
    };

/** Mirror of `ConnectionState` (`#[serde(tag="state", content="data")]`). */
export type ConnectionState =
  | { state: "Connecting" }
  | { state: "Connected" }
  | { state: "Reconnecting"; data: { in_seconds: number; reason: string } }
  | { state: "Disconnected"; data: { reason: string } };

/** Mirror of `JobState` (`#[serde(tag="state", content="reason")]`).
 * The `Failed` variant's `reason` is a bare string. */
export type JobState =
  | { state: "Idle" }
  | { state: "Preparing" }
  | { state: "Printing" }
  | { state: "Paused" }
  | { state: "Finished" }
  | { state: "Failed"; reason: string };

export interface JobProgress {
  file_name: string | null;
  current_layer: number | null;
  total_layers: number | null;
  percent: number | null;
  eta_seconds: number | null;
  state: JobState;
}

export interface TempReading {
  current: number;
  target: number;
}

export interface Temps {
  nozzles: TempReading[];
  bed: TempReading;
  chamber: TempReading | null;
}

/** Per-tray AMS filament identity. Color is RRGGBBAA hex without `#`. */
export interface AmsFilament {
  tray_type: string;
  color: string;
  sub_brand: string | null;
  multi_colors: string[];
  /** Bambu's vendor SKU (e.g. "GFA00" for PLA Basic). Null when
   *  the spool is untagged. The sync resolver matches this
   *  against bundled `FilamentFragmentSummary.filament_id`. */
  filament_id: string | null;
}

export interface AmsTray {
  id: number;
  identity: AmsFilament | null;
}

export interface AmsUnit {
  id: number;
  trays: AmsTray[];
}

export interface AmsState {
  units: AmsUnit[];
  active_slot: number | null;
}

export interface BambuExtra {
  mounted_plate: string | null;
  current_stage: string | null;
  print_error_code: number | null;
  /** Last non-zero Bambu err_code from a rejected command (84033543 =
   *  Developer Mode required); null when none. */
  command_error_code: number | null;
  fan_speed: number | null;
  ams: AmsState | null;
  /** External (PTFE-tube) spool — Bambu pushes this via
   *  `print.vt_tray` in MQTT alongside the AMS state. Carries the
   *  user-entered material + color even though the slot has no
   *  RFID. Sync maps this to the trailing Direct slot. */
  external_spool: AmsFilament | null;
}

export interface U1Filament {
  material_type: string;
  color: string;
}

export interface U1Extra {
  mounted_toolhead: number | null;
  toolhead_filaments: (U1Filament | null)[];
  current_stage: string | null;
  fan_speed: number | null;
}

/** Mirror of `DriverExtra` (`#[serde(tag="kind", content="data")]`). */
export type DriverExtra =
  | { kind: "Bambu"; data: BambuExtra }
  | { kind: "U1"; data: U1Extra };

/** Mirror of `PrinterStatus`. `last_updated` is Unix millis. */
export interface PrinterStatus {
  connection: ConnectionState;
  job: JobProgress | null;
  temps: Temps;
  extra: DriverExtra;
  last_updated: number;
}

export interface SendHandle {
  id: string;
  file_name: string;
}

/** Mirror of `PrinterCommand`. */
export type PrinterCommand = "Pause" | "Resume" | "Stop";

/** Payload of the `driver:status_update` Tauri event. */
export interface StatusUpdateEvent {
  driver_id: DriverId;
  status: PrinterStatus;
}

/** Payload of the `driver:upload_progress` Tauri event — emitted (throttled)
 *  while a send pushes the sliced bundle to the printer. `percent` is 0..100. */
export interface UploadProgress {
  driver_id: DriverId;
  file_name: string;
  sent: number;
  total: number;
  percent: number;
}
