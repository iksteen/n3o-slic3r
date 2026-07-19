//! The [`Driver`] trait + the small set of domain types that
//! cross the trait boundary (id, error, send payload, command).
//!
//! Phase 8 will lift drivers into out-of-process plugins. This
//! trait is shaped so a plugin can implement it without needing
//! a redesign — every method takes simple owned / borrowed
//! values that survive IPC serialization.

use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use super::ams::AmsMappingV2;
pub use crate::core::printer::instance::SendOptions;
use tokio::sync::watch;

use super::status::PrinterStatus;

/// Stable identifier for a registered driver instance. Allocated
/// by [`super::registry::DriverRegistry::register`] and stable
/// for the driver's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DriverId(pub u64);

impl fmt::Display for DriverId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "driver-{}", self.0)
    }
}

/// Which protocol/printer family a driver implements. Used by
/// the registry to decide what `DriverConfig` variant to expect +
/// by the frontend to pick the right credentials dialog.
///
/// Serializes as lowercase (`"bambu"` / `"u1"` / `"moonraker"`) — the
/// project uses lowercase identifiers across the wire and in authored
/// TOML (`model.toml::driver_kind`), matching `ConnectionInfo`'s
/// `snake_case` tagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverKind {
    Bambu,
    U1,
    /// Generic Klipper printer speaking vanilla Moonraker. The U1 is
    /// Moonraker-backed too but keeps its own kind for its bespoke
    /// webcam stack (pairing + mTLS monitor wake).
    Moonraker,
}

/// Per-driver connection configuration. The variant must match
/// the driver kind being registered; mismatches fail at
/// `register` time with a structured error rather than at first
/// `connect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum DriverConfig {
    Bambu {
        host: String,
        /// 8-char code shown on the printer LCD under network
        /// settings.
        access_code: String,
    },
    U1 {
        host: String,
        #[serde(default = "default_moonraker_port")]
        port: u16,
    },
    /// Generic Moonraker endpoint — same fields as U1 (which is a
    /// Moonraker printer with a vendor webcam stack on top).
    Moonraker {
        host: String,
        #[serde(default = "default_moonraker_port")]
        port: u16,
    },
}

fn default_moonraker_port() -> u16 {
    80
}

/// What the caller hands to [`Driver::send`]. Each variant maps
/// to the wire format the target printer expects — Bambu wants a
/// `.gcode.3mf` bundle, U1 wants raw G-code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum SendPayload {
    /// Bambu A1 mini: pre-built `.gcode.3mf` bundle. The bytes
    /// are the FTPS-uploaded body; `plate_id` is repeated for
    /// the MQTT command's `subtask_name` field. AMS routing
    /// fields match the shape BBS publishes — both `ams_mapping`
    /// (`[i8]`) and `ams_mapping2` (`[{ams_id, slot_id}]`) are
    /// required; firmware ignores the `.gcode.3mf` bundle's
    /// `ams_bindings` and falls back to the external PTFE-tube
    /// spool without both. Arrays are sized to the plate's
    /// materials list length (one entry per material, indexed by
    /// `material - 1`); see `ams_mapping_for_plate` for the
    /// encoding rules.
    Gcode3mf {
        bytes: Vec<u8>,
        plate_id: u32,
        /// Project+plate-derived, filename-safe basename for the FTPS upload /
        /// the printer-visible job name (`<file_basename>.gcode.3mf`).
        file_basename: String,
        use_ams: bool,
        ams_mapping: Vec<i8>,
        ams_mapping2: Vec<AmsMappingV2>,
        /// Per-print toggles for the MQTT `project_file` command
        /// (`bed_leveling` / `flow_cali` / `vibration_cali` / `timelapse`).
        options: SendOptions,
    },
    /// Moonraker-served printers: raw G-code body + the filename the
    /// printer should store it under (`<file_name>.gcode`).
    Gcode {
        bytes: Vec<u8>,
        file_name: String,
        /// `Some` for the Snapmaker U1: start via the vendor
        /// `SDCARD_PRINT_FILE_WITH_PARAMETERS` macro carrying the
        /// per-print toggles. `None` (generic Moonraker) starts the
        /// print in the upload request itself — no per-print option
        /// protocol exists there.
        u1_start: Option<U1StartOptions>,
    },
}

/// Per-print toggles + the print-usage facts the U1 firmware gates flow
/// calibration on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct U1StartOptions {
    pub options: SendOptions,
    /// Physical extruders this print uses (0-based; the sliced G-code's
    /// filament indices with nonzero usage — for a toolchanger those ARE
    /// the toolhead numbers). Sent as `FLOW_CALIBRATE_EXTRUDERS`.
    pub extruders_used: Vec<u8>,
    /// Per-extruder filament use in mm, index-aligned with the G-code's
    /// filament order. Sent as `FILAMENT_USED_MM`.
    pub filament_used_mm: Vec<f64>,
    /// Installed nozzle diameter per physical extruder, from the printer
    /// instance. Sent as `NOZZLE_DIAMETER_LIST`; the firmware validates
    /// it against the actual toolhead hardware for every used extruder
    /// (a missing list fails as `nozzle diameter mismatch: f_0 != e_…`).
    pub nozzle_diameters: Vec<f64>,
    /// The firmware's `extruder_map_table` as `(logical, physical)` pairs,
    /// one per logical slot. Sent as `MAP_TABLE`; the table is sticky on
    /// the printer, so we always send the full table to overwrite any
    /// stale mapping a prior session (or Snapmaker's own software) left.
    pub map_table: Vec<(u8, u8)>,
}

/// What [`Driver::send`] returns on success. The id correlates
/// the running job in subsequent status updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendHandle {
    pub id: String,
    pub file_name: String,
}

/// Command verbs every driver implements. Trait method
/// [`Driver::command`] dispatches on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrinterCommand {
    Pause,
    Resume,
    Stop,
}

/// Driver-side errors. `Display` impl yields a user-facing
/// message; the `String` payload of `Network`/`Auth`/`Protocol`/
/// `Other` carries the underlying detail for logs.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum DriverError {
    #[error("network error: {0}")]
    Network(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("driver is not connected")]
    NotConnected,
    #[error("{0}")]
    Other(String),
}

/// Upload-progress callback: `(bytes_sent, total_bytes)`, invoked as the
/// driver pushes the bundle to the printer. `Arc<dyn Fn ...>` (not a generic
/// param) keeps [`Driver`] object-safe and lets the closure clone into Bambu's
/// blocking FTPS task and U1's async upload stream.
pub type UploadProgressFn = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// The lifecycle + command surface shared by every printer
/// driver. See the module-level docs in [`super`] for the
/// design rationale.
///
/// Object-safe (`Send + Sync` super-trait bounds + no generic
/// methods) so the registry can store `Box<dyn Driver>`.
#[async_trait]
pub trait Driver: Send + Sync {
    /// The id the registry assigned at `register` time.
    fn id(&self) -> DriverId;

    /// Which kind of driver this is — frontend uses this to
    /// branch on the right per-driver UI.
    fn kind(&self) -> DriverKind;

    /// Establish the printer connection. Spawning of background
    /// tasks (rumqttc event loop, Moonraker WebSocket worker)
    /// happens here. Idempotent: connecting an already-connected
    /// driver is a no-op `Ok(())`.
    async fn connect(&mut self) -> Result<(), DriverError>;

    /// Tear down the connection cleanly. Idempotent: dropping an
    /// already-disconnected driver is a no-op.
    async fn disconnect(&mut self) -> Result<(), DriverError>;

    /// Latest snapshot. Cheap; reads the current value from the
    /// driver's internal `watch::Sender<PrinterStatus>`.
    fn status(&self) -> PrinterStatus;

    /// Subscribe to live status updates. The receiver fires
    /// (at most ~1 Hz, per the driver's rate-limiter) whenever
    /// the snapshot changes.
    fn subscribe_status(&self) -> watch::Receiver<PrinterStatus>;

    /// Upload + start a print. Returns a handle the caller can
    /// correlate against subsequent status updates.
    ///
    /// `on_progress(bytes_sent, total_bytes)` fires as the bundle is pushed to
    /// the printer — drivers report real upload progress through it. The caller
    /// throttles before surfacing it to the UI.
    ///
    /// Cancellation is the caller's job: every driver's `send` is fully async,
    /// so the command layer aborts an in-flight upload by dropping this future
    /// (raced against a cancel signal in `tokio::select!`).
    /// Takes `&self` (not `&mut self`): the impls clone the handles they need
    /// (client, status sender) and don't mutate driver state, so a long upload
    /// can run under a shared lock while `status`/`command` proceed.
    async fn send(
        &self,
        payload: SendPayload,
        on_progress: UploadProgressFn,
    ) -> Result<SendHandle, DriverError>;

    /// Pause / resume / stop the current print. State guards
    /// inside the impl block return `DriverError::Other` for
    /// invalid transitions (pause from IDLE, resume from
    /// RUNNING, etc.) without contacting the printer.
    async fn command(&self, cmd: PrinterCommand) -> Result<(), DriverError>;

    /// Write a filament identity back to an AMS tray. Only printers
    /// with a writable AMS (Bambu AMS lite) implement this; the
    /// default rejects it so a toolchanger (U1) or the test mock
    /// don't have to. Used to push a UI-edited non-RFID slot to the
    /// printer — RFID-detected slots are gated out upstream (a write
    /// would be stomped on the next read).
    async fn set_ams_filament(
        &self,
        _setting: AmsFilamentSetting,
    ) -> Result<(), DriverError> {
        Err(DriverError::Other(
            "this printer has no writable AMS".into(),
        ))
    }

    /// Run a standalone pressure-advance calibration on the active
    /// toolhead and return the measured K. Only printers with a
    /// firmware calibration routine (the U1's `FLOW_CALIBRATE`)
    /// implement this; the default rejects it. The call is long-running
    /// (heats + sweeps + purges — minutes), so callers should not block
    /// UI on it.
    async fn calibrate_pressure_advance(&self) -> Result<f64, DriverError> {
        Err(DriverError::Other(
            "this printer has no pressure-advance calibration".into(),
        ))
    }

    /// A handle for vendor JSON-RPC requests over this driver's live
    /// status connection (the U1 camera wake rides it, over MQTT or WS
    /// alike). `None` when the driver has no such control plane or isn't
    /// currently connected. Default: no control plane.
    fn control_plane(&self) -> Option<std::sync::Arc<dyn ControlPlane>> {
        None
    }
}

/// Fire-and-forget JSON-RPC over a driver's live connection. The
/// transport frames it (MQTT publish on `<sn>/request`, WebSocket text
/// message); responses are not surfaced — callers that need one don't fit
/// this hatch. Handles go stale when their session drops: sends fail and
/// the caller re-fetches via [`Driver::control_plane`].
#[async_trait]
pub trait ControlPlane: Send + Sync {
    async fn send_jsonrpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), DriverError>;
}

/// Parameters for writing one filament identity back to a Bambu AMS
/// (lite) tray. The tray is addressed as `(ams_id, tray_id)`; the
/// rest is the spool identity the printer should store. Mirrors the
/// `ams_filament_setting` MQTT command fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmsFilamentSetting {
    pub ams_id: u8,
    pub tray_id: u8,
    /// Slot position within the AMS unit. For a regular AMS (the A1
    /// mini's AMS lite, `ams_id <= 3`) this equals `tray_id`. BambuStudio
    /// sends it as a distinct field alongside `tray_id`; firmware can
    /// reject the command without it (per bambuddy's packet captures).
    pub slot_id: u8,
    /// Bambu vendor SKU (`tray_info_idx`, e.g. `"GFL99"`).
    pub tray_info_idx: String,
    /// Material type (`"PLA"`, `"PETG"`, …).
    pub tray_type: String,
    /// Sub-brand label (e.g. `"PLA Basic"`); empty when unknown.
    pub tray_sub_brands: String,
    /// Spool color as Bambu `RRGGBBAA` hex8 (no `#`).
    pub tray_color: String,
    pub nozzle_temp_min: i32,
    pub nozzle_temp_max: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time sanity: the trait must be object-safe so the
    /// registry can store `Box<dyn Driver>`. If this line ever
    /// fails to compile, we've added a generic method or a
    /// non-`Self` receiver — those break dyn dispatch.
    #[test]
    fn driver_trait_is_object_safe() {
        fn _accept(_: &mut dyn Driver) {}
    }

    #[test]
    fn send_payload_serde_round_trips_both_variants() {
        let bambu = SendPayload::Gcode3mf {
            bytes: vec![1, 2, 3],
            plate_id: 1,
            file_basename: "MyPrint_Lid".into(),
            use_ams: true,
            ams_mapping: vec![-1, 0],
            ams_mapping2: vec![
                AmsMappingV2 {
                    ams_id: 255,
                    slot_id: 0,
                },
                AmsMappingV2 {
                    ams_id: 0,
                    slot_id: 0,
                },
            ],
            options: SendOptions::default(),
        };
        let s = serde_json::to_string(&bambu).unwrap();
        let back: SendPayload = serde_json::from_str(&s).unwrap();
        match back {
            SendPayload::Gcode3mf {
                plate_id,
                bytes,
                file_basename,
                use_ams,
                ams_mapping,
                ams_mapping2,
                options,
            } => {
                assert_eq!(plate_id, 1);
                assert_eq!(bytes, vec![1, 2, 3]);
                assert_eq!(file_basename, "MyPrint_Lid");
                assert!(use_ams);
                assert_eq!(ams_mapping, vec![-1, 0]);
                assert_eq!(
                    ams_mapping2,
                    vec![
                        AmsMappingV2 {
                            ams_id: 255,
                            slot_id: 0
                        },
                        AmsMappingV2 {
                            ams_id: 0,
                            slot_id: 0
                        },
                    ]
                );
                assert_eq!(options, SendOptions::default());
            }
            _ => panic!("variant"),
        }

        let u1 = SendPayload::Gcode {
            bytes: vec![4, 5],
            file_name: "x.gcode".into(),
            u1_start: Some(U1StartOptions {
                options: SendOptions::default(),
                extruders_used: vec![0, 1],
                filament_used_mm: vec![500.0, 600.0],
                nozzle_diameters: vec![0.4, 0.4],
                map_table: vec![(0, 0), (1, 1)],
            }),
        };
        let s = serde_json::to_string(&u1).unwrap();
        let back: SendPayload = serde_json::from_str(&s).unwrap();
        match back {
            SendPayload::Gcode {
                file_name, u1_start, ..
            } => {
                assert_eq!(file_name, "x.gcode");
                assert_eq!(u1_start.unwrap().extruders_used, vec![0, 1]);
            }
            _ => panic!("variant"),
        }
    }

    #[test]
    fn driver_error_displays_user_facing_message() {
        let e = DriverError::Auth("bad access code".into());
        assert_eq!(e.to_string(), "authentication failed: bad access code");
    }

    #[test]
    fn driver_config_serde_tag_kind() {
        let cfg = DriverConfig::U1 {
            host: "192.168.1.42".into(),
            port: 80,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        // Frontend sees `{ "kind": "U1", "data": { … } }`.
        assert!(s.contains("\"kind\":\"U1\""));
        assert!(s.contains("\"port\":80"));
    }
}
