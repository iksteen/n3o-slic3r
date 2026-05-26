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

pub use crate::core::slice::pre_slice_gate::AmsMappingV2;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverKind {
    Bambu,
    U1,
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
        /// Device serial. Optional — when omitted, PR-7a-2's
        /// connect path probes the peer cert CN.
        #[serde(default)]
        serial: Option<String>,
    },
    U1 {
        host: String,
        #[serde(default = "default_u1_port")]
        port: u16,
        /// Probed via `/machine/system_info` at connect time
        /// when omitted.
        #[serde(default)]
        serial: Option<String>,
    },
}

fn default_u1_port() -> u16 {
    80
}

/// What the caller hands to [`Driver::send`]. Each variant maps
/// to the wire format the target printer expects — Bambu wants a
/// `.gcode.3mf` bundle (PR-3-10), U1 wants raw G-code.
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
        use_ams: bool,
        ams_mapping: Vec<i8>,
        ams_mapping2: Vec<AmsMappingV2>,
    },
    /// Snapmaker U1: raw G-code body + the filename the printer
    /// should store it under (`<file_name>.gcode`).
    Gcode { bytes: Vec<u8>, file_name: String },
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
    async fn send(&mut self, payload: SendPayload) -> Result<SendHandle, DriverError>;

    /// Pause / resume / stop the current print. State guards
    /// inside the impl block return `DriverError::Other` for
    /// invalid transitions (pause from IDLE, resume from
    /// RUNNING, etc.) without contacting the printer.
    async fn command(&mut self, cmd: PrinterCommand) -> Result<(), DriverError>;
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
            use_ams: true,
            ams_mapping: vec![-1, 0],
            ams_mapping2: vec![
                AmsMappingV2 { ams_id: 255, slot_id: 0 },
                AmsMappingV2 { ams_id: 0, slot_id: 0 },
            ],
        };
        let s = serde_json::to_string(&bambu).unwrap();
        let back: SendPayload = serde_json::from_str(&s).unwrap();
        match back {
            SendPayload::Gcode3mf { plate_id, bytes, use_ams, ams_mapping, ams_mapping2 } => {
                assert_eq!(plate_id, 1);
                assert_eq!(bytes, vec![1, 2, 3]);
                assert!(use_ams);
                assert_eq!(ams_mapping, vec![-1, 0]);
                assert_eq!(ams_mapping2, vec![
                    AmsMappingV2 { ams_id: 255, slot_id: 0 },
                    AmsMappingV2 { ams_id: 0, slot_id: 0 },
                ]);
            }
            _ => panic!("variant"),
        }

        let u1 = SendPayload::Gcode {
            bytes: vec![4, 5],
            file_name: "x.gcode".into(),
        };
        let s = serde_json::to_string(&u1).unwrap();
        let back: SendPayload = serde_json::from_str(&s).unwrap();
        match back {
            SendPayload::Gcode { file_name, .. } => assert_eq!(file_name, "x.gcode"),
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
            serial: None,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        // Frontend sees `{ "kind": "U1", "data": { … } }`.
        assert!(s.contains("\"kind\":\"U1\""));
        assert!(s.contains("\"port\":80"));
    }
}
