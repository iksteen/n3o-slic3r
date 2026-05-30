//! Convert a Moonraker `printer.objects.subscribe` status map into
//! a vendor-neutral [`PrinterStatus`] with the U1's per-toolhead
//! detail in [`U1Extra`].
//!
//! Ported from `iksteen/bambu-overlay` `src/snapmaker/report.rs`.
//! Differences from the overlay:
//!
//! - Output shape is [`PrinterStatus`] (the n3o driver-trait
//!   contract), not the overlay's `PrinterReport`. State names map
//!   into [`JobState`]'s enum variants rather than free-form
//!   strings.
//! - Per-toolhead filaments land on [`U1Extra::toolhead_filaments`]
//!   in 0-indexed extruder-position order (one Option per slot;
//!   displayed as T1..T4 in the UI), matching the [`U1Filament`]
//!   shape the driver trait reserves.
//! - Per-toolhead temperatures land on [`Temps::nozzles`] in the
//!   same 0-indexed order. Missing extruders are dropped, not
//!   padded; the frontend reads `nozzles.len()` to know how many
//!   toolheads the printer reports.

use std::time::SystemTime;

use serde_json::{Map, Value};

use super::moonraker::{extruders, get_f64, get_print_info_i64, get_string};
use crate::core::driver::status::{
    ConnectionState, DriverExtra, JobProgress, JobState, PrinterStatus, TempReading, Temps,
    U1Extra, U1Filament,
};

/// Build the full [`PrinterStatus`] snapshot from a Moonraker
/// status map. `connection` is passed in because the WS layer
/// owns connection lifecycle — the decoder only knows about
/// payload state.
#[allow(dead_code)] // consumed by PR-7b-5's U1Driver worker
pub(super) fn decode(status: &Map<String, Value>, connection: ConnectionState) -> PrinterStatus {
    PrinterStatus {
        connection,
        job: decode_job(status),
        temps: decode_temps(status),
        extra: DriverExtra::U1(decode_extra(status)),
        last_updated: SystemTime::now(),
    }
}

fn decode_job(status: &Map<String, Value>) -> Option<JobProgress> {
    let state = decode_state(status)?;
    let file_name = get_string(status, "print_stats", "filename")
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_owned);
    let percent = get_f64(status, "display_status", "progress")
        .map(|fraction| (fraction * 100.0).clamp(0.0, 100.0) as f32);
    let current_layer = get_print_info_i64(status, "current_layer")
        .filter(|n| *n >= 0)
        .and_then(|n| u32::try_from(n).ok());
    let total_layers = get_print_info_i64(status, "total_layer")
        .filter(|n| *n >= 0)
        .and_then(|n| u32::try_from(n).ok());
    Some(JobProgress {
        file_name,
        current_layer,
        total_layers,
        percent,
        eta_seconds: estimate_eta_seconds(status, percent),
        state,
    })
}

/// Estimate remaining print time for the U1.
///
/// Klipper/Moonraker exposes no native remaining-time field — only
/// `print_stats.print_duration` (elapsed seconds) and
/// `display_status.progress` (a 0..1 fraction). We linearly extrapolate
/// the remaining time from those, the same fallback Mainsail/Fluidd use
/// for their "file" estimate: `remaining = elapsed * (100/pct - 1)`.
///
/// It's only a rough forecast (assumes uniform pace) — early readings
/// run high and converge as the print progresses — but showing a rough
/// number beats showing nothing. We surface it from the first real
/// progress tick: `print_duration` is `0` during heating (Klipper only
/// starts it at first extrusion), so the `elapsed > 0` guard naturally
/// keeps the field at "—" through warmup without a progress floor. The
/// only gates are `pct > 0` (the ratio divides by it) and a `< 99.5%`
/// ceiling, above which there's nothing useful left to show and
/// rounding noise dominates. Idle / no `print_duration` → `None` → "—".
fn estimate_eta_seconds(status: &Map<String, Value>, percent: Option<f32>) -> Option<u64> {
    let pct = percent?;
    if pct <= 0.0 || pct >= 99.5 {
        return None;
    }
    let elapsed = get_f64(status, "print_stats", "print_duration")?;
    if elapsed <= 0.0 {
        return None;
    }
    let remaining = elapsed * (100.0 / pct as f64 - 1.0);
    if remaining.is_finite() && remaining >= 0.0 {
        Some(remaining.round() as u64)
    } else {
        None
    }
}

/// Maps `print_stats.state` (Klipper's lower-case strings) into
/// our [`JobState`] enum. Returns `None` when the field is absent
/// or empty — caller then leaves `PrinterStatus.job = None`,
/// which the UI renders as "no job".
fn decode_state(status: &Map<String, Value>) -> Option<JobState> {
    let raw = get_string(status, "print_stats", "state")?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(match raw.to_ascii_lowercase().as_str() {
        "standby" => JobState::Idle,
        "printing" => JobState::Printing,
        "paused" => JobState::Paused,
        "complete" => JobState::Finished,
        "cancelled" | "error" => {
            // `print_stats.message` (when present) carries the
            // failure reason — surface it so the UI can show "Print
            // failed: filament runout" instead of just "Failed".
            let reason = get_string(status, "print_stats", "message")
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .unwrap_or(raw)
                .to_owned();
            JobState::Failed(reason)
        }
        // Unknown states (newer Klipper might add some) get
        // surfaced verbatim under `Failed` rather than silently
        // dropped to Idle — easier to triage.
        other => JobState::Failed(format!("unknown Klipper state: {other}")),
    })
}

fn decode_temps(status: &Map<String, Value>) -> Temps {
    let bed = TempReading {
        current: get_f64(status, "heater_bed", "temperature").unwrap_or(0.0) as f32,
        target: get_f64(status, "heater_bed", "target").unwrap_or(0.0) as f32,
    };
    let ex_map = extruders(status);
    let mut indices: Vec<usize> = ex_map.keys().copied().collect();
    indices.sort();
    let nozzles = indices
        .into_iter()
        .map(|i| TempReading {
            current: ex_map
                .get(&i)
                .and_then(|v| v.get("temperature").and_then(Value::as_f64))
                .unwrap_or(0.0) as f32,
            target: ex_map
                .get(&i)
                .and_then(|v| v.get("target").and_then(Value::as_f64))
                .unwrap_or(0.0) as f32,
        })
        .collect();
    Temps { nozzles, bed, chamber: None }
}

fn decode_extra(status: &Map<String, Value>) -> U1Extra {
    U1Extra {
        mounted_toolhead: active_tool_index(status).and_then(|i| u8::try_from(i).ok()),
        toolhead_filaments: decode_toolhead_filaments(status),
        current_stage: None,
        fan_speed: get_f64(status, "fan", "speed").map(|f| (f * 100.0).clamp(0.0, 100.0) as f32),
    }
}

/// Klipper's currently-selected extruder lives in `toolhead.extruder`,
/// formatted as `"extruder"` (index 0) or `"extruderN"` (index N>=1).
fn active_tool_index(status: &Map<String, Value>) -> Option<usize> {
    let name = get_string(status, "toolhead", "extruder")?.trim();
    if name == "extruder" {
        return Some(0);
    }
    name.strip_prefix("extruder")?.parse::<usize>().ok()
}

/// Per-toolhead filament from Snapmaker's `print_task_config` object.
/// Returns an entry per color slot in declaration order; transparent
/// or empty slots come back as `None`. Empty vector when the printer
/// isn't reporting `print_task_config` (e.g. idle, no job, or non-
/// Snapmaker Klipper firmware).
fn decode_toolhead_filaments(status: &Map<String, Value>) -> Vec<Option<U1Filament>> {
    let Some(task) = status.get("print_task_config").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(colors) = task.get("filament_color_rgba").and_then(Value::as_array) else {
        return Vec::new();
    };
    let types = task.get("filament_type").and_then(Value::as_array);
    colors
        .iter()
        .enumerate()
        .map(|(idx, color_value)| {
            let color = color_value
                .as_str()
                .and_then(normalize_rgba)?;
            let material_type = types
                .and_then(|list| list.get(idx))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Filament")
                .to_owned();
            Some(U1Filament {
                material_type,
                color,
            })
        })
        .collect()
}

/// Validate Snapmaker's RRGGBBAA hex string and return it uppercase
/// without `#` (matches our `U1Filament.color` convention, which
/// mirrors PR-7a's `AmsFilament.color`). Empty / fully-transparent
/// slots (`#00000000`) return `None` so the caller treats them as
/// unbound.
fn normalize_rgba(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_start_matches('#');
    if trimmed.len() < 6 {
        return None;
    }
    let rgb = &trimmed[..6];
    // Sentinel for "no filament loaded" — fully-transparent black.
    if rgb.eq_ignore_ascii_case("000000") && trimmed.get(6..8) == Some("00") {
        return None;
    }
    // Reject non-hex bodies; the printer should never emit them
    // but firmware bugs are firmware bugs.
    u32::from_str_radix(rgb, 16).ok()?;
    // Preserve alpha when present (8 chars); fall back to RGB only
    // when the printer reports 6.
    let normalized = if trimmed.len() >= 8 {
        let alpha = &trimmed[6..8];
        u32::from_str_radix(alpha, 16).ok()?;
        format!("{rgb}{alpha}").to_ascii_uppercase()
    } else {
        rgb.to_ascii_uppercase()
    };
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            _ => panic!("status must be an object"),
        }
    }

    fn decoded(value: Value) -> PrinterStatus {
        decode(&status(value), ConnectionState::Connected)
    }

    // ---- state mapping ----

    #[test]
    fn decode_maps_each_klipper_state_variant() {
        assert!(matches!(
            decoded(json!({ "print_stats": { "state": "standby" } })).job.unwrap().state,
            JobState::Idle,
        ));
        assert!(matches!(
            decoded(json!({ "print_stats": { "state": "printing" } })).job.unwrap().state,
            JobState::Printing,
        ));
        assert!(matches!(
            decoded(json!({ "print_stats": { "state": "paused" } })).job.unwrap().state,
            JobState::Paused,
        ));
        assert!(matches!(
            decoded(json!({ "print_stats": { "state": "complete" } })).job.unwrap().state,
            JobState::Finished,
        ));
    }

    #[test]
    fn decode_surfaces_failure_message_on_error_state() {
        let p = decoded(json!({
            "print_stats": { "state": "error", "message": "thermal runaway" }
        }));
        match p.job.unwrap().state {
            JobState::Failed(msg) => assert_eq!(msg, "thermal runaway"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn decode_falls_back_to_state_string_when_failure_message_missing() {
        // No `message` field — the raw state name is the next-best
        // signal so the UI shows "Print failed: cancelled" rather
        // than "Print failed: <empty>".
        let p = decoded(json!({
            "print_stats": { "state": "cancelled" }
        }));
        match p.job.unwrap().state {
            JobState::Failed(msg) => assert_eq!(msg, "cancelled"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn decode_returns_no_job_when_state_absent() {
        let p = decoded(json!({}));
        assert!(p.job.is_none());
    }

    #[test]
    fn unknown_klipper_state_surfaces_as_failed() {
        let p = decoded(json!({
            "print_stats": { "state": "warming_the_house" }
        }));
        match p.job.unwrap().state {
            JobState::Failed(msg) => assert!(msg.contains("warming_the_house"), "got {msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ---- job progress fields ----

    #[test]
    fn decode_extracts_filename_layer_progress() {
        let p = decoded(json!({
            "print_stats": {
                "state": "printing",
                "filename": "Cube.gcode",
                "info": { "current_layer": 12, "total_layer": 80 },
            },
            "display_status": { "progress": 0.4275 },
        }));
        let job = p.job.unwrap();
        assert_eq!(job.file_name.as_deref(), Some("Cube.gcode"));
        assert_eq!(job.current_layer, Some(12));
        assert_eq!(job.total_layers, Some(80));
        // 0.4275 * 100 → 42.75 (f64 → f32 round-trip is exact here).
        assert!((job.percent.unwrap() - 42.75).abs() < 1e-3);
    }

    #[test]
    fn empty_filename_becomes_none() {
        let p = decoded(json!({
            "print_stats": { "state": "standby", "filename": "" }
        }));
        assert!(p.job.unwrap().file_name.is_none());
    }

    #[test]
    fn progress_clamps_outside_0_to_100() {
        // Display sometimes reports >1.0 right after a 0→print
        // transition. We clamp so the UI never sees -5% or 150%.
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "display_status": { "progress": 1.5 }
        }));
        assert_eq!(p.job.unwrap().percent, Some(100.0));
    }

    #[test]
    fn eta_extrapolated_from_elapsed_and_progress() {
        // Klipper has no native forecast, so we linearly extrapolate
        // from print_duration + progress: 300s elapsed at 25% →
        // remaining = 300 * (100/25 - 1) = 900s.
        let p = decoded(json!({
            "print_stats": { "state": "printing", "print_duration": 300.0 },
            "display_status": { "progress": 0.25 },
        }));
        assert_eq!(p.job.unwrap().eta_seconds, Some(900));
    }

    #[test]
    fn eta_none_without_progress() {
        // print_duration present but no progress fraction → can't
        // extrapolate → None (renders as "—").
        let p = decoded(json!({
            "print_stats": { "state": "printing", "print_duration": 300.0 },
        }));
        assert!(p.job.unwrap().eta_seconds.is_none());
    }

    #[test]
    fn eta_shown_from_first_real_progress_tick() {
        // No progress floor: as soon as there's positive progress AND
        // elapsed time, show a (rough, high) estimate rather than "—".
        // 5s at 0.1% → 5 * (100/0.1 - 1) = 4995s.
        let early = decoded(json!({
            "print_stats": { "state": "printing", "print_duration": 5.0 },
            "display_status": { "progress": 0.001 },
        }));
        assert_eq!(early.job.unwrap().eta_seconds, Some(4995));
    }

    #[test]
    fn eta_none_during_heating_and_above_ceiling() {
        // Heating: progress > 0 but print_duration still 0 (Klipper
        // starts it at first extrusion) → "—" until printing begins.
        let heating = decoded(json!({
            "print_stats": { "state": "printing", "print_duration": 0.0 },
            "display_status": { "progress": 0.002 },
        }));
        assert!(heating.job.unwrap().eta_seconds.is_none());
        // Near-complete: nothing useful left to show.
        let late = decoded(json!({
            "print_stats": { "state": "printing", "print_duration": 3600.0 },
            "display_status": { "progress": 0.999 },
        }));
        assert!(late.job.unwrap().eta_seconds.is_none());
    }

    #[test]
    fn eta_none_when_idle_no_elapsed() {
        // Standby / no print_duration → None even if a stale progress
        // value lingers.
        let p = decoded(json!({
            "print_stats": { "state": "standby" },
            "display_status": { "progress": 0.4 },
        }));
        // Idle still produces a job (state present), but no ETA.
        assert!(p.job.unwrap().eta_seconds.is_none());
    }

    #[test]
    fn negative_layer_count_is_dropped() {
        // Defensive: invalid integers shouldn't poison the UI as
        // huge u32s.
        let p = decoded(json!({
            "print_stats": {
                "state": "printing",
                "info": { "current_layer": -1, "total_layer": -42 }
            }
        }));
        let job = p.job.unwrap();
        assert!(job.current_layer.is_none());
        assert!(job.total_layers.is_none());
    }

    // ---- temps ----

    #[test]
    fn decode_bed_and_per_toolhead_temps() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "extruder":  { "temperature": 215.0, "target": 220.0 },
            "extruder1": { "temperature":  24.0, "target":   0.0 },
            "extruder2": { "temperature":  25.5, "target": 200.0 },
            "extruder3": { "temperature":  26.0, "target":   0.0 },
            "heater_bed": { "temperature": 60.5, "target": 60.0 },
        }));
        assert_eq!(p.temps.bed.current, 60.5);
        assert_eq!(p.temps.bed.target, 60.0);
        assert_eq!(p.temps.nozzles.len(), 4);
        assert_eq!(p.temps.nozzles[0].current, 215.0);
        assert_eq!(p.temps.nozzles[0].target, 220.0);
        assert_eq!(p.temps.nozzles[2].current, 25.5);
        assert_eq!(p.temps.nozzles[2].target, 200.0);
        // Chamber sensor is missing in U1 hardware — always None.
        assert!(p.temps.chamber.is_none());
    }

    #[test]
    fn missing_extruder_dropped_from_nozzles_vec() {
        // Only T0 + T2 reported (T1, T3 absent). We don't pad.
        let p = decoded(json!({
            "print_stats": { "state": "standby" },
            "extruder":  { "temperature": 24.0 },
            "extruder2": { "temperature": 25.0 },
        }));
        assert_eq!(p.temps.nozzles.len(), 2);
    }

    // ---- extra: mounted toolhead + filaments ----

    #[test]
    fn mounted_toolhead_indexes_from_zero() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "toolhead": { "extruder": "extruder2" },
        }));
        match p.extra {
            DriverExtra::U1(extra) => assert_eq!(extra.mounted_toolhead, Some(2)),
            _ => panic!("U1 driver must publish U1 extra"),
        }
    }

    #[test]
    fn mounted_toolhead_handles_extruder_zero_name() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "toolhead": { "extruder": "extruder" },
        }));
        match p.extra {
            DriverExtra::U1(extra) => assert_eq!(extra.mounted_toolhead, Some(0)),
            _ => panic!("U1 driver must publish U1 extra"),
        }
    }

    #[test]
    fn toolhead_filaments_decode_color_and_type_per_slot() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "print_task_config": {
                "filament_color_rgba": ["E72F1DFF", "F4C032FF", "080A0DFF", "E2DEDBFF"],
                "filament_type": ["PLA", "PLA", "PETG", "TPU"],
            }
        }));
        match p.extra {
            DriverExtra::U1(extra) => {
                assert_eq!(extra.toolhead_filaments.len(), 4);
                let t0 = extra.toolhead_filaments[0].as_ref().unwrap();
                assert_eq!(t0.color, "E72F1DFF");
                assert_eq!(t0.material_type, "PLA");
                let t2 = extra.toolhead_filaments[2].as_ref().unwrap();
                assert_eq!(t2.material_type, "PETG");
                assert_eq!(extra.toolhead_filaments[3].as_ref().unwrap().material_type, "TPU");
            }
            _ => panic!("U1 driver must publish U1 extra"),
        }
    }

    #[test]
    fn fully_transparent_slot_decodes_as_none() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "print_task_config": {
                "filament_color_rgba": ["FF0000FF", "00000000"],
                "filament_type": ["PLA", ""],
            }
        }));
        match p.extra {
            DriverExtra::U1(extra) => {
                assert!(extra.toolhead_filaments[0].is_some());
                assert!(extra.toolhead_filaments[1].is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn toolhead_filaments_empty_without_print_task_config() {
        // No `print_task_config` (idle printer) → empty vec, not
        // a Vec<None>. UI reads `.is_empty()` to know "no info".
        let p = decoded(json!({
            "print_stats": { "state": "standby" }
        }));
        match p.extra {
            DriverExtra::U1(extra) => assert!(extra.toolhead_filaments.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn filament_type_missing_defaults_to_generic_label() {
        // print_task_config exists but filament_type array absent.
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "print_task_config": {
                "filament_color_rgba": ["E72F1DFF"]
            }
        }));
        match p.extra {
            DriverExtra::U1(extra) => {
                let t0 = extra.toolhead_filaments[0].as_ref().unwrap();
                assert_eq!(t0.material_type, "Filament");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn fan_speed_in_percent_clamped() {
        let p = decoded(json!({
            "print_stats": { "state": "printing" },
            "fan": { "speed": 0.6 }
        }));
        match p.extra {
            DriverExtra::U1(extra) => assert_eq!(extra.fan_speed, Some(60.0)),
            _ => panic!(),
        }
        let over = decoded(json!({
            "print_stats": { "state": "printing" },
            "fan": { "speed": 1.5 }
        }));
        match over.extra {
            DriverExtra::U1(extra) => assert_eq!(extra.fan_speed, Some(100.0)),
            _ => panic!(),
        }
    }

    // ---- rgba normalizer ----

    #[test]
    fn normalize_rgba_strips_hash_and_uppercases_rrggbbaa() {
        assert_eq!(normalize_rgba("e72f1dff").as_deref(), Some("E72F1DFF"));
        assert_eq!(normalize_rgba("#E72F1DFF").as_deref(), Some("E72F1DFF"));
    }

    #[test]
    fn normalize_rgba_accepts_rgb_only_input() {
        // U1 reports RRGGBBAA, but defensive: accept 6-hex if it
        // ever appears (e.g. from a sliced project's metadata).
        assert_eq!(normalize_rgba("E72F1D").as_deref(), Some("E72F1D"));
    }

    #[test]
    fn normalize_rgba_rejects_short_or_non_hex() {
        assert!(normalize_rgba("").is_none());
        assert!(normalize_rgba("ABC").is_none());
        assert!(normalize_rgba("GG0000FF").is_none());
        assert!(normalize_rgba("00000000").is_none()); // transparent sentinel
    }

    // ---- connection wiring ----

    #[test]
    fn decode_threads_caller_supplied_connection_state() {
        let p = decode(
            &status(json!({ "print_stats": { "state": "standby" } })),
            ConnectionState::Reconnecting { in_seconds: 5, reason: "boom".into() },
        );
        match p.connection {
            ConnectionState::Reconnecting { in_seconds, reason } => {
                assert_eq!(in_seconds, 5);
                assert_eq!(reason, "boom");
            }
            _ => panic!("decoder must not touch the connection field"),
        }
    }

    // ---- file-based fixtures captured from a real Snapmaker U1 ----
    //
    // The inline `json!` tests above cover individual field shapes; the
    // fixture tests cross-check that the decoder handles *real-world
    // payload envelopes* the printer actually emits — including objects
    // the inline cases don't bother to model (gcode_move, virtual_sdcard,
    // print_task_config's full key set, etc.). See
    // `tests/fixtures/u1-moonraker/README.md` for capture provenance.

    const FIXTURES_DIR: &str = "tests/fixtures/u1-moonraker";

    fn load_fixture(name: &str) -> Value {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURES_DIR)
            .join(name);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse fixture {}: {e}", path.display()))
    }

    /// Subscribe-reply `result.status` map — the initial full-snapshot
    /// shape `MoonrakerSession::connect` decodes via PR-7b-3's pipeline.
    fn subscribe_initial() -> Map<String, Value> {
        let fixture = load_fixture("subscribe_response.json");
        fixture["result"]["status"]
            .as_object()
            .expect("subscribe fixture missing result.status")
            .clone()
    }

    /// Pull the delta update map out of a `notify_status_update` fixture
    /// (`params[0]` — same path `MoonrakerSession::next_status` uses).
    fn notify_update(name: &str) -> Map<String, Value> {
        let fixture = load_fixture(name);
        fixture["params"][0]
            .as_object()
            .expect("notify fixture missing params[0]")
            .clone()
    }

    /// Inline mirror of `MoonrakerSession::merge_status`. Pattern matches
    /// `moonraker.rs::tests::merge` — both copies are tiny and keep the
    /// tests honest by not depending on private production internals.
    fn merge_status(into: &mut Map<String, Value>, update: &Map<String, Value>) {
        for (key, value) in update {
            match (into.get_mut(key), value) {
                (Some(existing), Value::Object(patch)) if existing.is_object() => {
                    let existing = existing.as_object_mut().expect("checked is_object");
                    for (subkey, subvalue) in patch {
                        existing.insert(subkey.clone(), subvalue.clone());
                    }
                }
                _ => {
                    into.insert(key.clone(), value.clone());
                }
            }
        }
    }

    #[test]
    fn fixture_subscribe_decodes_full_snapshot() {
        let snap = decode(&subscribe_initial(), ConnectionState::Connected);
        // Capture happened immediately after a finished print, so state
        // is `complete` rather than `standby` — exercises the Finished
        // path, which the inline tests cover with a single-field json
        // and which this fixture confirms also holds with the full
        // surrounding payload.
        let job = snap.job.expect("job present");
        assert!(
            matches!(job.state, JobState::Finished),
            "expected Finished, got {:?}",
            job.state,
        );
        assert_eq!(job.file_name.as_deref(), Some("plate-1.gcode"));
        // U1 reports all 4 extruders regardless of which is docked.
        assert_eq!(snap.temps.nozzles.len(), 4);
        assert!(snap.temps.bed.current > 0.0);
        match snap.extra {
            DriverExtra::U1(extra) => {
                // The post-print snapshot still reports the last-docked
                // toolhead — extruder1 in this capture.
                assert_eq!(extra.mounted_toolhead, Some(1));
            }
            _ => panic!("U1 driver must publish U1 extra"),
        }
    }

    #[test]
    fn fixture_layer_advance_merge_surfaces_current_layer() {
        // Real layer-advance frames don't carry `print_stats.state` —
        // only the delta. Merging into the subscribe baseline (which
        // does have state) gives the decoder a complete picture, same
        // as the production pipeline does over the WS.
        let mut baseline = subscribe_initial();
        let update = notify_update("notify_layer_advance.json");
        merge_status(&mut baseline, &update);
        let snap = decode(&baseline, ConnectionState::Connected);
        let job = snap.job.expect("job present after merge");
        // Fixture was chosen mid-print (current_layer > 5); the
        // total_layer survives from the subscribe baseline because
        // the delta only patches current_layer.
        assert!(job.current_layer.unwrap_or(0) > 5);
        assert_eq!(job.total_layers, Some(100));
    }

    #[test]
    fn fixture_toolchange_merge_surfaces_mounted_toolhead() {
        let mut baseline = subscribe_initial();
        let update = notify_update("notify_toolchange.json");
        merge_status(&mut baseline, &update);
        let snap = decode(&baseline, ConnectionState::Connected);
        match snap.extra {
            DriverExtra::U1(extra) => {
                // The toolchange fixture flips toolhead.extruder to
                // "extruder1"; baseline already had "extruder1" too,
                // but the test pins the post-merge value to confirm
                // the decoder still reads it after a delta merge that
                // touches the toolhead object.
                assert_eq!(extra.mounted_toolhead, Some(1));
            }
            _ => panic!("U1 driver must publish U1 extra"),
        }
    }

    #[test]
    fn fixture_eta_extrapolates_from_real_print_duration() {
        // The captured subscribe fixture is a real U1 payload with
        // print_stats.print_duration present (≈919s). Force it mid-
        // print (state=printing, progress 0.5) and confirm the decoder
        // extrapolates remaining from the real elapsed value:
        // 919.377 * (100/50 - 1) = 919.377 → rounds to 919.
        let mut baseline = subscribe_initial();
        merge_status(
            &mut baseline,
            &status(json!({
                "print_stats": { "state": "printing" },
                "display_status": { "progress": 0.5 },
            })),
        );
        let job = decode(&baseline, ConnectionState::Connected)
            .job
            .expect("job present");
        assert_eq!(job.eta_seconds, Some(919));
    }
}
