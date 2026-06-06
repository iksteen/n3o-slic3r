//! [`PrinterStatus`] + the supporting types. The shape every
//! driver fills + the typed extras union for driver-specific UI.
//!
//! Per-driver extras live in [`DriverExtra`] with one variant
//! per driver. Drivers populate their own variant; the frontend
//! reads common fields generically and branches on
//! `status.extra` for AMS / toolhead detail.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// The shared status snapshot every driver publishes via
/// `subscribe_status` / `status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterStatus {
    pub connection: ConnectionState,
    pub job: Option<JobProgress>,
    pub temps: Temps,
    pub extra: DriverExtra,
    #[serde(with = "serde_systemtime")]
    pub last_updated: SystemTime,
}

impl PrinterStatus {
    /// What a freshly-registered driver publishes before it has
    /// connected for the first time.
    pub fn disconnected_for(extra: DriverExtra) -> Self {
        Self {
            connection: ConnectionState::Disconnected {
                reason: "not yet connected".into(),
            },
            job: None,
            temps: Temps::default(),
            extra,
            last_updated: SystemTime::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data")]
pub enum ConnectionState {
    Connecting,
    Connected,
    /// Currently waiting before the next reconnect attempt.
    /// `reason` carries the failure that triggered the backoff (the
    /// driver's last connect/poll error) so the UI and the
    /// test-connection command can report why rather than a bare
    /// "reconnecting".
    Reconnecting {
        in_seconds: u32,
        reason: String,
    },
    Disconnected {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobProgress {
    pub file_name: Option<String>,
    /// `None` when the printer doesn't natively expose it (the
    /// U1 / standard Klipper case — see PR-7b-3's known gap).
    pub current_layer: Option<u32>,
    pub total_layers: Option<u32>,
    pub percent: Option<f32>,
    pub eta_seconds: Option<u64>,
    pub state: JobState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason")]
pub enum JobState {
    Idle,
    /// Heating / calibrating / homing before the print starts — an active
    /// pre-print phase (Bambu reports it as gcode_state "PREPARE"). Not Idle
    /// (the printer is busy) and not yet Printing (no layers laid down).
    Preparing,
    Printing,
    Paused,
    Finished,
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Temps {
    /// One entry per active nozzle. A1 mini has one; U1 has up
    /// to four (per active toolhead).
    pub nozzles: Vec<TempReading>,
    pub bed: TempReading,
    pub chamber: Option<TempReading>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TempReading {
    pub current: f32,
    pub target: f32,
}

/// Driver-specific status extras. One variant per driver.
/// PR-7a-4 / PR-7b-3 populate the inner fields; this ticket
/// (PR-7a-1) ships them as empty placeholders so the trait +
/// registry compile against the public shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum DriverExtra {
    Bambu(BambuExtra),
    U1(U1Extra),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BambuExtra {
    /// What plate type the printer reports having mounted. Feeds
    /// the BuildPlate cascade layer (PR-7a-3 wires the cascade
    /// re-resolution).
    pub mounted_plate: Option<String>,
    /// Bambu firmware "current stage" code; surfaced for
    /// diagnostics. Free-form string — PR-7a-3 maps known codes.
    pub current_stage: Option<String>,
    pub print_error_code: Option<i32>,
    /// Last non-zero Bambu `err_code` from a rejected command (84033543
    /// = Developer Mode required); `None` once a command succeeds.
    pub command_error_code: Option<i32>,
    pub fan_speed: Option<f32>,
    /// Populated by PR-7a-4. `None` until that ticket lands or
    /// when the printer reports no AMS.
    pub ams: Option<AmsState>,
    /// External spool (the rear PTFE-tube "virtual tray"). Bambu
    /// reports it in MQTT as `print.vt_tray`, alongside the AMS
    /// state. Carries the user-entered material + color even though
    /// the slot itself has no RFID — sync (PR-7c-2) uses it to
    /// keep the Ext slot in step with the printer's display.
    /// `None` until populated.
    pub external_spool: Option<AmsFilament>,
}

/// AMS state placeholder. Real shape lands in PR-7a-4 with all
/// the per-tray / per-unit fields; here we keep it as an opaque
/// container so the trait surface is stable now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsState {
    pub units: Vec<AmsUnit>,
    pub active_slot: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsUnit {
    pub id: u8,
    pub trays: Vec<AmsTray>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsTray {
    pub id: u8,
    pub identity: Option<AmsFilament>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AmsFilament {
    pub tray_type: String,
    /// `RRGGBBAA` hex, no `#`.
    pub color: String,
    pub sub_brand: Option<String>,
    pub multi_colors: Vec<String>,
    /// Bambu's vendor SKU for the spool (e.g. "GFA00" for PLA
    /// Basic), reported as `tray_info_idx` in the MQTT push.
    /// PR-7c-2's sync resolver matches this against bundled
    /// `FilamentFragmentSummary.filament_id` for an exact identity
    /// lookup. `None` for trays that don't report one (untagged
    /// spool, or older firmware).
    pub filament_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct U1Extra {
    /// Currently-mounted toolhead index (0..3). `None` if no
    /// toolhead is mounted (rare; usually means just-powered
    /// state).
    pub mounted_toolhead: Option<u8>,
    pub toolhead_filaments: Vec<Option<U1Filament>>,
    pub current_stage: Option<String>,
    pub fan_speed: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct U1Filament {
    pub material_type: String,
    /// `RRGGBBAA` hex, no `#`.
    pub color: String,
}

/// Serde helper for `SystemTime` round-trips as Unix millis.
/// `SystemTime` doesn't have a stable serde impl that survives
/// JSON round-trip; we want one that does.
mod serde_systemtime {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let millis = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        millis.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let millis = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_extra_serde_tag_kind() {
        let extra = DriverExtra::Bambu(BambuExtra::default());
        let s = serde_json::to_string(&extra).unwrap();
        assert!(s.contains("\"kind\":\"Bambu\""));

        let extra = DriverExtra::U1(U1Extra::default());
        let s = serde_json::to_string(&extra).unwrap();
        assert!(s.contains("\"kind\":\"U1\""));
    }

    #[test]
    fn printer_status_serde_round_trip_preserves_last_updated() {
        let status = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        let s = serde_json::to_string(&status).unwrap();
        let back: PrinterStatus = serde_json::from_str(&s).unwrap();
        // millisecond precision lost intentionally — sub-ms
        // resolution isn't useful for status snapshots.
        assert_eq!(
            back.last_updated
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            status
                .last_updated
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        );
    }

    #[test]
    fn job_state_failed_carries_reason() {
        let s = JobState::Failed("nozzle clog".into());
        let json = serde_json::to_string(&s).unwrap();
        let back: JobState = serde_json::from_str(&json).unwrap();
        match back {
            JobState::Failed(r) => assert_eq!(r, "nozzle clog"),
            _ => panic!("variant"),
        }
    }
}
