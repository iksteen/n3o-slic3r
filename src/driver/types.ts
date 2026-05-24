// Wire-shape mirrors of the Rust driver types (PR-7a-7).
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

/** Mirror of `DriverKind`. External-tagged on the wire. */
export type DriverKind = "Bambu" | "U1";

/** Mirror of `DriverConfig` (`#[serde(tag="kind", content="data")]`). */
export type DriverConfig =
  | {
      kind: "Bambu";
      data: {
        host: string;
        access_code: string;
        serial: string | null;
      };
    }
  | {
      kind: "U1";
      data: {
        host: string;
        port: number;
        serial: string | null;
      };
    };

/** Mirror of `ConnectionState` (`#[serde(tag="state", content="data")]`). */
export type ConnectionState =
  | { state: "Connecting" }
  | { state: "Connected" }
  | { state: "Reconnecting"; data: { in_seconds: number } }
  | { state: "Disconnected"; data: { reason: string } };

/** Mirror of `JobState` (`#[serde(tag="state", content="reason")]`).
 * The `Failed` variant's `reason` is a bare string. */
export type JobState =
  | { state: "Idle" }
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
  fan_speed: number | null;
  ams: AmsState | null;
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

export interface DriverSummary {
  id: DriverId;
  kind: DriverKind;
  connection: ConnectionState;
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
