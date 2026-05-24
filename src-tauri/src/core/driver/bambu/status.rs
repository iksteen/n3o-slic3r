//! Bambu MQTT report payload → typed [`PrinterStatus`] (PR-7a-3).
//!
//! Field shape and merge semantics are a faithful port of
//! `bambu-overlay/src/bambu/models.rs`. Extensions (chamber temp,
//! mounted plate, current stage, print error) are added because
//! the PR-7a-3 ticket calls for them and our slicing-cascade
//! ties to `bed_type`.
//!
//! Bambu's API has a quirk where some fields arrive as JSON
//! numbers and others as JSON strings ("50" vs 50). The
//! [`de`] module's helpers tolerate both — overlay does the
//! same.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use crate::core::driver::status::{
    ConnectionState, DriverExtra, JobProgress, JobState,
    PrinterStatus, TempReading,
};
#[cfg(test)]
use crate::core::driver::status::BambuExtra;

mod de {
    //! Number-or-string-coerced optional fields. Bambu's API
    //! sends scalar values as strings sometimes (`"50"`) and as
    //! numbers other times (`50`); both must deserialize.

    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    pub(super) fn optional_string<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<String>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::String(s) if s.is_empty() => Ok(None),
            Value::String(s) => Ok(Some(s)),
            Value::Number(n) => Ok(Some(n.to_string())),
            Value::Bool(b) => Ok(Some(b.to_string())),
            _ => Ok(None),
        }
    }

    pub(super) fn optional_i64<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<i64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::Number(n) => Ok(n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))),
            Value::String(s) => Ok(s.trim().parse::<i64>().ok()),
            _ => Ok(None),
        }
    }

    pub(super) fn optional_f64<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<f64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::Number(n) => Ok(n.as_f64()),
            Value::String(s) => Ok(s.trim().parse::<f64>().ok()),
            _ => Ok(None),
        }
    }
}

/// Wire-shape mirror of Bambu's `device/<id>/report` MQTT
/// payload. Always wrapped in a top-level `{ "print": { … } }`
/// (which is what [`BambuMessage`] handles).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct BambuReport {
    // --- Job ---
    #[serde(default, rename = "gcode_state", deserialize_with = "de::optional_string")]
    pub status_text: Option<String>,
    #[serde(default, rename = "mc_percent", deserialize_with = "de::optional_f64")]
    pub percent: Option<f64>,
    #[serde(default, rename = "gcode_file", deserialize_with = "de::optional_string")]
    pub filename: Option<String>,
    #[serde(default, rename = "layer_num", deserialize_with = "de::optional_i64")]
    pub layer_num: Option<i64>,
    #[serde(default, rename = "total_layer_num", deserialize_with = "de::optional_i64")]
    pub total_layer_num: Option<i64>,
    #[serde(
        default,
        rename = "mc_remaining_time",
        deserialize_with = "de::optional_f64"
    )]
    pub remaining_minutes: Option<f64>,

    // --- Temps ---
    #[serde(default, rename = "nozzle_temper", deserialize_with = "de::optional_f64")]
    pub nozzle_temper: Option<f64>,
    #[serde(default, rename = "nozzle_target_temper", deserialize_with = "de::optional_f64")]
    pub nozzle_target_temper: Option<f64>,
    #[serde(default, rename = "bed_temper", deserialize_with = "de::optional_f64")]
    pub bed_temper: Option<f64>,
    #[serde(default, rename = "bed_target_temper", deserialize_with = "de::optional_f64")]
    pub bed_target_temper: Option<f64>,
    /// Not in bambu-overlay's model; added because PR-7a-3 calls
    /// it out + enclosed printers will surface it.
    #[serde(default, rename = "chamber_temper", deserialize_with = "de::optional_f64")]
    pub chamber_temper: Option<f64>,

    // --- Bambu extras ---
    /// Mounted build plate, e.g. `"cool_plate"`, `"textured_pei"`.
    /// Not in bambu-overlay's model — added so we can feed the
    /// BuildPlate cascade layer.
    #[serde(default, rename = "bed_type", deserialize_with = "de::optional_string")]
    pub bed_type: Option<String>,
    /// Bambu firmware "current stage" code. Free-form; PR-7a-3
    /// surfaces as-is for diagnostics.
    #[serde(default, rename = "stg_cur", deserialize_with = "de::optional_i64")]
    pub current_stage: Option<i64>,
    #[serde(default, rename = "print_error", deserialize_with = "de::optional_i64")]
    pub print_error: Option<i64>,
    #[serde(default, rename = "cooling_fan_speed", deserialize_with = "de::optional_f64")]
    pub fan_speed: Option<f64>,

    // --- AMS — populated by PR-7a-4 ---
    #[serde(default)]
    pub ams: Option<serde_json::Value>,
}

impl BambuReport {
    /// Apply a delta to the receiver. Last-write-wins for scalar
    /// fields (when patch is `Some`); AMS is replaced verbatim
    /// when patch is `Some` (PR-7a-4 layers spool-aware merge on
    /// top once it adds the typed AMS shape).
    pub fn merge(&mut self, patch: BambuReport) {
        macro_rules! lww {
            ($($f:ident),+ $(,)?) => {
                $( if patch.$f.is_some() { self.$f = patch.$f; } )+
            };
        }
        lww!(
            status_text,
            percent,
            filename,
            layer_num,
            total_layer_num,
            remaining_minutes,
            nozzle_temper,
            nozzle_target_temper,
            bed_temper,
            bed_target_temper,
            chamber_temper,
            bed_type,
            current_stage,
            print_error,
            fan_speed,
            ams,
        );
    }
}

/// Top-level wrapper Bambu wraps the report in. We `serde(default)`
/// the `print` field so messages without it (e.g. firmware
/// keepalives) parse without error.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BambuMessage {
    #[serde(default)]
    pub print: BambuReport,
}

/// Decode raw MQTT payload bytes into a typed `BambuMessage`.
/// Returns the parsed value or a parse error string.
pub fn parse_message(bytes: &[u8]) -> Result<BambuMessage, String> {
    serde_json::from_slice(bytes)
        .map_err(|e| format!("bambu report parse: {e}"))
}

/// Apply a `BambuReport` delta to the live `PrinterStatus`
/// snapshot the watch sender holds. Scalar fields update by
/// last-write-wins; `last_updated` always advances.
pub fn merge_into(snapshot: &mut PrinterStatus, msg: BambuReport) {
    let now = SystemTime::now();
    snapshot.last_updated = now;

    if let Some(ref s) = msg.status_text {
        // Status field stays in JobProgress, but JobProgress is
        // an Option so we synthesize one on first sight.
        let job = snapshot.job.get_or_insert(JobProgress {
            file_name: None,
            current_layer: None,
            total_layers: None,
            percent: None,
            eta_seconds: None,
            state: JobState::Idle,
        });
        job.state = map_state(s);
    }
    if let Some(file) = msg.filename {
        snapshot
            .job
            .get_or_insert(default_job())
            .file_name = Some(file);
    }
    if let Some(n) = msg.layer_num {
        snapshot.job.get_or_insert(default_job()).current_layer = Some(n as u32);
    }
    if let Some(n) = msg.total_layer_num {
        snapshot.job.get_or_insert(default_job()).total_layers = Some(n as u32);
    }
    if let Some(p) = msg.percent {
        snapshot.job.get_or_insert(default_job()).percent = Some(p as f32);
    }
    if let Some(m) = msg.remaining_minutes {
        // Bambu's `mc_remaining_time` is in minutes per their
        // protocol notes — convert to seconds for `PrinterStatus`.
        snapshot.job.get_or_insert(default_job()).eta_seconds =
            Some((m * 60.0) as u64);
    }

    // Temps.
    let temps = &mut snapshot.temps;
    if temps.nozzles.is_empty() {
        temps.nozzles.push(TempReading::default());
    }
    if let Some(c) = msg.nozzle_temper {
        temps.nozzles[0].current = c as f32;
    }
    if let Some(t) = msg.nozzle_target_temper {
        temps.nozzles[0].target = t as f32;
    }
    if let Some(c) = msg.bed_temper {
        temps.bed.current = c as f32;
    }
    if let Some(t) = msg.bed_target_temper {
        temps.bed.target = t as f32;
    }
    if let Some(c) = msg.chamber_temper {
        temps.chamber.get_or_insert(TempReading::default()).current = c as f32;
    }

    // Driver-specific extras.
    if let DriverExtra::Bambu(ref mut extra) = snapshot.extra {
        if let Some(s) = msg.bed_type {
            extra.mounted_plate = Some(s);
        }
        if let Some(s) = msg.current_stage {
            extra.current_stage = Some(s.to_string());
        }
        if let Some(e) = msg.print_error {
            extra.print_error_code = Some(e as i32);
        }
        if let Some(f) = msg.fan_speed {
            extra.fan_speed = Some(f as f32);
        }
        // AMS payload is forwarded as opaque JSON until PR-7a-4
        // adds typed parsing. We don't replace extra.ams here —
        // PR-7a-4 will plumb it through the typed shape.
    }
}

fn default_job() -> JobProgress {
    JobProgress {
        file_name: None,
        current_layer: None,
        total_layers: None,
        percent: None,
        eta_seconds: None,
        state: JobState::Idle,
    }
}

fn map_state(s: &str) -> JobState {
    match s {
        "IDLE" | "FINISH" if s == "FINISH" => JobState::Finished,
        "IDLE" => JobState::Idle,
        "RUNNING" => JobState::Printing,
        "PAUSE" => JobState::Paused,
        "FINISH" => JobState::Finished,
        "FAILED" => JobState::Failed("printer reported FAILED".into()),
        other => JobState::Failed(format!("unknown Bambu state: {other}")),
    }
}

/// Status worker task. Drains the raw-payload channel + merges
/// each parsed delta into the snapshot. Rate-limits UI updates
/// to ≤1 Hz by accumulating merges and flushing on a tokio
/// `interval` tick. Trace-logs every raw message regardless for
/// diagnostics.
pub async fn run_worker(
    mut raw_rx: mpsc::Receiver<Vec<u8>>,
    status_tx: watch::Sender<PrinterStatus>,
) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Snapshot we accumulate deltas into between ticks.
    let mut pending: Option<PrinterStatus> = None;
    let mut dirty = false;

    loop {
        tokio::select! {
            maybe_bytes = raw_rx.recv() => {
                let Some(bytes) = maybe_bytes else {
                    tracing::debug!("bambu raw_rx closed; status worker exiting");
                    return;
                };
                tracing::trace!(len = bytes.len(), "bambu raw report");
                match parse_message(&bytes) {
                    Ok(msg) => {
                        // Initialize pending from current status if
                        // first message.
                        let snapshot = pending.get_or_insert_with(
                            || status_tx.borrow().clone(),
                        );
                        merge_into(snapshot, msg.print);
                        // Connection state can't go backwards from
                        // Connected; reset to Connected on first
                        // successful merge in case backoff left a
                        // stale Reconnecting state.
                        if !matches!(snapshot.connection, ConnectionState::Connected) {
                            snapshot.connection = ConnectionState::Connected;
                        }
                        dirty = true;
                    }
                    Err(e) => tracing::warn!(error = %e, "bambu report parse failed"),
                }
            }
            _ = interval.tick() => {
                if dirty {
                    if let Some(s) = pending.clone() {
                        let _ = status_tx.send(s);
                    }
                    dirty = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_handles_string_or_number_for_layer_num() {
        let msg_num: BambuMessage = serde_json::from_value(json!({
            "print": { "layer_num": 42 }
        })).unwrap();
        assert_eq!(msg_num.print.layer_num, Some(42));

        let msg_str: BambuMessage = serde_json::from_value(json!({
            "print": { "layer_num": "42" }
        })).unwrap();
        assert_eq!(msg_str.print.layer_num, Some(42));
    }

    #[test]
    fn parse_tolerates_unknown_fields() {
        let body = br#"{"print":{"layer_num":3,"new_firmware_field":"value"}}"#;
        let msg = parse_message(body).expect("forward-compat parse");
        assert_eq!(msg.print.layer_num, Some(3));
    }

    #[test]
    fn parse_tolerates_missing_print_wrapper() {
        let body = br#"{}"#;
        let msg = parse_message(body).expect("empty parse");
        assert!(msg.print.layer_num.is_none());
    }

    #[test]
    fn merge_last_write_wins_for_present_scalar() {
        let mut a = BambuReport {
            percent: Some(10.0),
            filename: Some("old.3mf".into()),
            ..Default::default()
        };
        let b = BambuReport {
            percent: Some(50.0),
            // filename omitted in patch
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.percent, Some(50.0));
        assert_eq!(a.filename.as_deref(), Some("old.3mf")); // preserved
    }

    #[test]
    fn merge_into_populates_job_progress() {
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(
            BambuExtra::default(),
        ));
        let report = BambuReport {
            status_text: Some("RUNNING".into()),
            layer_num: Some(5),
            total_layer_num: Some(100),
            percent: Some(5.0),
            remaining_minutes: Some(120.0),
            ..Default::default()
        };
        merge_into(&mut snap, report);
        let job = snap.job.expect("job populated");
        assert_eq!(job.state, JobState::Printing);
        assert_eq!(job.current_layer, Some(5));
        assert_eq!(job.total_layers, Some(100));
        assert_eq!(job.percent, Some(5.0));
        assert_eq!(job.eta_seconds, Some(7200)); // 120 min → sec
    }

    #[test]
    fn merge_into_populates_temps_and_bambu_extra() {
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(
            BambuExtra::default(),
        ));
        let report = BambuReport {
            nozzle_temper: Some(205.5),
            nozzle_target_temper: Some(210.0),
            bed_temper: Some(60.2),
            bed_target_temper: Some(60.0),
            chamber_temper: Some(35.0),
            bed_type: Some("textured_pei".into()),
            fan_speed: Some(75.0),
            current_stage: Some(2),
            print_error: Some(0),
            ..Default::default()
        };
        merge_into(&mut snap, report);
        assert!((snap.temps.nozzles[0].current - 205.5).abs() < 0.001);
        assert!((snap.temps.nozzles[0].target - 210.0).abs() < 0.001);
        assert!((snap.temps.bed.current - 60.2).abs() < 0.01);
        assert!((snap.temps.chamber.unwrap().current - 35.0).abs() < 0.01);
        match snap.extra {
            DriverExtra::Bambu(e) => {
                assert_eq!(e.mounted_plate.as_deref(), Some("textured_pei"));
                assert_eq!(e.fan_speed, Some(75.0));
                assert_eq!(e.current_stage.as_deref(), Some("2"));
                assert_eq!(e.print_error_code, Some(0));
            }
            _ => panic!("expected Bambu extra"),
        }
    }

    #[test]
    fn merge_into_advances_last_updated() {
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(
            BambuExtra::default(),
        ));
        let before = snap.last_updated;
        std::thread::sleep(std::time::Duration::from_millis(5));
        merge_into(&mut snap, BambuReport::default());
        assert!(
            snap.last_updated > before,
            "last_updated must advance on each merge"
        );
    }

    #[test]
    fn state_mapping_covers_known_strings() {
        assert!(matches!(map_state("IDLE"), JobState::Idle));
        assert!(matches!(map_state("RUNNING"), JobState::Printing));
        assert!(matches!(map_state("PAUSE"), JobState::Paused));
        assert!(matches!(map_state("FINISH"), JobState::Finished));
        match map_state("FAILED") {
            JobState::Failed(_) => {}
            other => panic!("expected Failed, got {other:?}"),
        }
        match map_state("UNKNOWN_FIRMWARE_STATE") {
            JobState::Failed(reason) => assert!(reason.contains("UNKNOWN_FIRMWARE_STATE")),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
