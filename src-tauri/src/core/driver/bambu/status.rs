//! Bambu MQTT report payload → typed [`PrinterStatus`].
//!
//! Field shape and merge semantics are a faithful port of
//! `bambu-overlay/src/bambu/models.rs`. Extensions (chamber temp,
//! mounted plate, current stage, print error) are added because
//! our slicing-cascade ties to `bed_type`.
//!
//! Bambu's API has a quirk where some fields arrive as JSON
//! numbers and others as JSON strings ("50" vs 50). The
//! [`de`] module's helpers tolerate both — overlay does the
//! same.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

#[cfg(test)]
use crate::core::driver::status::BambuExtra;
use crate::core::driver::status::{
    ConnectionState, DriverExtra, JobProgress, JobState, PrinterStatus, TempReading,
};

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

    /// Like [`optional_string`] but **presence-preserving**: a present
    /// empty string stays `Some("")` rather than collapsing to `None`.
    /// Combined with `#[serde(default)]`, this lets the AMS merge tell a
    /// field the printer *omitted* (absent → `None`, keep cached) from
    /// one it *sent empty* (`Some("")`, overwrite/clear) — Bambu's
    /// incremental pushes omit unchanged fields, so the distinction is
    /// load-bearing. Only `null` maps to `None` alongside absence.
    pub(super) fn present_string<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<String>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s)),
            Value::Number(n) => Ok(Some(n.to_string())),
            Value::Bool(b) => Ok(Some(b.to_string())),
            _ => Ok(None),
        }
    }

    /// Presence-preserving vector deserializer — distinguishes an
    /// omitted array (absent → `None`, keep cached) from a present one
    /// (`Some(vec)`, overwrite, even when empty).
    pub(super) fn present_vec<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<String>>, D::Error> {
        Ok(Some(Vec::<String>::deserialize(d)?))
    }

    pub(super) fn optional_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::Number(n) => Ok(n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))),
            Value::String(s) => Ok(s.trim().parse::<i64>().ok()),
            _ => Ok(None),
        }
    }

    pub(super) fn optional_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
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
    #[serde(
        default,
        rename = "gcode_state",
        deserialize_with = "de::optional_string"
    )]
    pub status_text: Option<String>,
    #[serde(default, rename = "mc_percent", deserialize_with = "de::optional_f64")]
    pub percent: Option<f64>,
    #[serde(
        default,
        rename = "gcode_file",
        deserialize_with = "de::optional_string"
    )]
    pub filename: Option<String>,
    #[serde(default, rename = "layer_num", deserialize_with = "de::optional_i64")]
    pub layer_num: Option<i64>,
    #[serde(
        default,
        rename = "total_layer_num",
        deserialize_with = "de::optional_i64"
    )]
    pub total_layer_num: Option<i64>,
    #[serde(
        default,
        rename = "mc_remaining_time",
        deserialize_with = "de::optional_f64"
    )]
    pub remaining_minutes: Option<f64>,

    // --- Temps ---
    #[serde(
        default,
        rename = "nozzle_temper",
        deserialize_with = "de::optional_f64"
    )]
    pub nozzle_temper: Option<f64>,
    #[serde(
        default,
        rename = "nozzle_target_temper",
        deserialize_with = "de::optional_f64"
    )]
    pub nozzle_target_temper: Option<f64>,
    #[serde(default, rename = "bed_temper", deserialize_with = "de::optional_f64")]
    pub bed_temper: Option<f64>,
    #[serde(
        default,
        rename = "bed_target_temper",
        deserialize_with = "de::optional_f64"
    )]
    pub bed_target_temper: Option<f64>,
    /// Not in bambu-overlay's model; added because enclosed
    /// printers surface it.
    #[serde(
        default,
        rename = "chamber_temper",
        deserialize_with = "de::optional_f64"
    )]
    pub chamber_temper: Option<f64>,

    // --- Bambu extras ---
    /// Mounted build plate, e.g. `"cool_plate"`, `"textured_pei"`.
    /// Not in bambu-overlay's model — added so we can feed the
    /// BuildPlate cascade layer.
    #[serde(default, rename = "bed_type", deserialize_with = "de::optional_string")]
    pub bed_type: Option<String>,
    /// Bambu firmware "current stage" code. Free-form; surfaced
    /// as-is for diagnostics.
    #[serde(default, rename = "stg_cur", deserialize_with = "de::optional_i64")]
    pub current_stage: Option<i64>,
    #[serde(default, rename = "print_error", deserialize_with = "de::optional_i64")]
    pub print_error: Option<i64>,
    /// Non-zero on a command echo = the printer rejected our command;
    /// 84033543 = Developer Mode off. Absent / 0 on normal reports.
    #[serde(default, rename = "err_code", deserialize_with = "de::optional_i64")]
    pub err_code: Option<i64>,
    #[serde(
        default,
        rename = "cooling_fan_speed",
        deserialize_with = "de::optional_f64"
    )]
    pub fan_speed: Option<f64>,

    // --- AMS ---
    #[serde(default)]
    pub ams: Option<RawAmsState>,
    /// Virtual tray = external spool (the rear PTFE-tube feed).
    /// Same shape as an AMS tray; the printer holds the
    /// user-entered material + color even though the slot itself
    /// has no RFID. Driver sync consumes it for the trailing
    /// Direct slot.
    #[serde(default)]
    pub vt_tray: Option<RawAmsTray>,
}

/// Wire-shape mirror of the `print.ams` sub-object. Decoded
/// into the typed [`AmsState`] in [`status::DriverExtra::Bambu`].
/// Ported from `bambu-overlay/src/bambu/models.rs:181-200`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct RawAmsState {
    /// Flat slot index of the active tray:
    /// `unit_id * 4 + tray_id`. Decoded into a separate field on
    /// the typed shape so consumers don't need to know the
    /// encoding.
    #[serde(default, rename = "tray_now", deserialize_with = "de::optional_i64")]
    pub tray_now: Option<i64>,
    #[serde(default)]
    pub ams: Vec<RawAmsUnit>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct RawAmsUnit {
    #[serde(default, deserialize_with = "de::optional_i64")]
    pub id: Option<i64>,
    #[serde(default)]
    pub tray: Vec<RawAmsTray>,
}

// Every string field uses `de::present_string` (not `optional_string`)
// so a present-but-empty value survives as `Some("")`, distinct from an
// omitted field (`None`). `merge_in` keys on that distinction; the
// `to_typed` lowering (`tray_identity`) still filters empties/transparent
// colors when it builds the user-facing `AmsFilament`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct RawAmsTray {
    #[serde(default, deserialize_with = "de::optional_i64")]
    pub id: Option<i64>,
    #[serde(
        default,
        rename = "tray_type",
        deserialize_with = "de::present_string"
    )]
    pub material: Option<String>,
    /// Bambu reports an empty/unloaded color as `"00000000"` (fully
    /// transparent black). Stored verbatim; `tray_identity` treats
    /// transparent-black as "no color" when lowering.
    #[serde(
        default,
        rename = "tray_color",
        deserialize_with = "de::present_string"
    )]
    pub color: Option<String>,
    /// Bambu's spool-specific identifier — varies by firmware
    /// path. Stored as-is. (Overlay reads `tray_sub_brands` —
    /// note plural — we accept either.)
    #[serde(
        default,
        rename = "tray_sub_brands",
        alias = "tray_sub_brand",
        deserialize_with = "de::present_string"
    )]
    pub sub_brand: Option<String>,
    /// Multi-color spool colors, populated for variegated
    /// filaments. `None` = omitted; `Some(vec)` = present (possibly
    /// empty for a solid spool).
    #[serde(default, deserialize_with = "de::present_vec")]
    pub cols: Option<Vec<String>>,
    /// Bambu's vendor SKU for the spool (e.g. "GFA00" for PLA
    /// Basic), or a `P`-prefixed local/custom preset id. Untagged
    /// spools still report a real value (a generic SKU or preset),
    /// not an empty string. The sync resolver uses this to look up
    /// the bundled fragment exactly.
    #[serde(
        default,
        rename = "tray_info_idx",
        deserialize_with = "de::present_string"
    )]
    pub tray_info_idx: Option<String>,
    /// RFID tag id of the loaded spool — present only when the AMS
    /// read a tagged (genuine Bambu) spool. 16 hex chars;
    /// `"0000000000000000"` (or empty) means no tag, i.e. a
    /// manually-set / third-party spool. Unlike `tray_info_idx`
    /// (set by both an RFID read and a manual filament pick), this is
    /// the reliable "auto-detected via RFID" discriminator.
    #[serde(default, rename = "tag_uid", deserialize_with = "de::present_string")]
    pub tag_uid: Option<String>,
}

impl RawAmsTray {
    /// Field-level merge for a tray patch — used for both AMS trays
    /// and the external `vt_tray`. BBL sends incremental pushes that
    /// carry only the changed field — e.g. `{"tray_color":"AC95D5FF"}`
    /// on a recolor, or `{"id":"0"}` mid-transition — so a wholesale
    /// replace would drop everything the last full push established.
    ///
    /// **Presence is the only rule**: a field the patch *carries*
    /// (`Some`, thanks to `de::present_string` keeping empties)
    /// overwrites the cached value — even when empty, because an empty
    /// value is a real state ("this spool has no sub-brand / no tag").
    /// A field the patch *omits* (`None`) is left untouched. No
    /// emptiness or transparent-black heuristics here — that
    /// interpretation belongs to `tray_identity`, which filters them
    /// when lowering to the user-facing `AmsFilament`. Conflating
    /// "omitted" with "sent empty" was the long-standing bug: a spool
    /// swap that legitimately cleared a field (e.g. dropped an RFID
    /// tag) left the stale value cached.
    pub fn merge_in(&mut self, patch: RawAmsTray) {
        if patch.id.is_some() {
            self.id = patch.id;
        }
        if patch.material.is_some() {
            self.material = patch.material;
        }
        if patch.color.is_some() {
            self.color = patch.color;
        }
        if patch.sub_brand.is_some() {
            self.sub_brand = patch.sub_brand;
        }
        if patch.cols.is_some() {
            self.cols = patch.cols;
        }
        if patch.tray_info_idx.is_some() {
            self.tray_info_idx = patch.tray_info_idx;
        }
        if patch.tag_uid.is_some() {
            self.tag_uid = patch.tag_uid;
        }
    }
}

impl RawAmsUnit {
    /// Per-tray field-merge. Each cached tray is patched field-wise by
    /// the tray at its position (see [`RawAmsTray::merge_in`]), so a
    /// placeholder push leaves real spool data intact without a gate;
    /// trays beyond the cached length are appended to reserve the
    /// position. `id` last-write-wins on `is_some`.
    pub fn merge_in(&mut self, patch: RawAmsUnit) {
        if patch.id.is_some() {
            self.id = patch.id;
        }
        for (i, patch_tray) in patch.tray.into_iter().enumerate() {
            match self.tray.get_mut(i) {
                Some(self_tray) => self_tray.merge_in(patch_tray),
                None => self.tray.push(patch_tray),
            }
        }
    }
}

impl RawAmsState {
    /// Merge a patch — `tray_now` last-write-wins, then each unit is
    /// field-merged at its position (units beyond the cached length
    /// are appended). Placeholder pushes (BBL's startup-time empty-tray
    /// reports) advance the active slot but leave the cached spool
    /// identities intact, because the per-field merge skips empty
    /// values — no separate placeholder gate needed.
    ///
    /// The original wholesale-replace-plus-gate approach came from
    /// `iksteen/machin3d-overlay` commit `dcf6b26350` ("Hopefully not
    /// lose AMS data during print startup"); field-merge preserves the
    /// same guarantee structurally and also survives partial single-
    /// field pushes (e.g. an in-place recolor) that the gate could not.
    pub fn merge_in(&mut self, patch: RawAmsState) {
        if patch.tray_now.is_some() {
            self.tray_now = patch.tray_now;
        }
        for (i, patch_unit) in patch.ams.into_iter().enumerate() {
            match self.ams.get_mut(i) {
                Some(self_unit) => self_unit.merge_in(patch_unit),
                None => self.ams.push(patch_unit),
            }
        }
    }

    /// Lower into the typed shape `BambuExtra` exposes.
    pub fn to_typed(&self) -> crate::core::driver::status::AmsState {
        use crate::core::driver::status::{AmsState, AmsTray, AmsUnit};
        let active_slot = self.tray_now.and_then(|n| u8::try_from(n).ok());
        let units = self
            .ams
            .iter()
            .enumerate()
            .map(|(idx, u)| AmsUnit {
                id: u.id.and_then(|i| u8::try_from(i).ok()).unwrap_or(idx as u8),
                trays: u
                    .tray
                    .iter()
                    .enumerate()
                    .map(|(tidx, t)| AmsTray {
                        id: t
                            .id
                            .and_then(|i| u8::try_from(i).ok())
                            .unwrap_or(tidx as u8),
                        identity: tray_identity(t),
                    })
                    .collect(),
            })
            .collect();
        AmsState { units, active_slot }
    }
}

/// `ams.tray_now` value meaning the external spool (`vt_tray`) is the
/// engaged filament path. AMS trays are `0..N`; `254` is the external
/// spool; `255` is "nothing engaged".
const EXTERNAL_SPOOL_TRAY_ID: i64 = 254;

fn tray_identity(t: &RawAmsTray) -> Option<crate::core::driver::status::AmsFilament> {
    use crate::core::driver::status::AmsFilament;
    let material = t
        .material
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let color = t
        .color
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|c| !is_transparent_black(c));
    let sub_brand = t
        .sub_brand
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let multi_colors: Vec<String> = t
        .cols
        .iter()
        .flatten()
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty() && !is_transparent_black(c))
        .collect();
    let filament_id = t
        .tray_info_idx
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    // Stored as-is (trimmed, non-empty); the all-zeros "no tag" case is
    // handled by `rfid_detected`, the single source of truth for the
    // RFID predicate. tag_uid never makes a tray "occupied" on its own.
    let tag_uid = t
        .tag_uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if material.is_none()
        && color.is_none()
        && sub_brand.is_none()
        && multi_colors.is_empty()
        && filament_id.is_none()
    {
        return None;
    }
    Some(AmsFilament {
        tray_type: material.unwrap_or("").to_owned(),
        color: color.unwrap_or("").to_owned(),
        sub_brand,
        multi_colors,
        filament_id,
        tag_uid,
    })
}

fn is_transparent_black(color: &str) -> bool {
    let normalized = color.trim().trim_start_matches('#');
    if normalized.len() < 6 {
        return false;
    }
    normalized[..6].eq_ignore_ascii_case("000000")
        && normalized.get(6..8).map(|a| a == "00").unwrap_or(true)
}

impl BambuReport {
    /// Apply a delta to the receiver. Last-write-wins for scalar
    /// fields (when patch is `Some`); AMS uses a presence-based
    /// per-tray merge ([`RawAmsState::merge_in`] →
    /// [`RawAmsTray::merge_in`]): a field the push carries overwrites,
    /// a field it omits is kept. BBL's incremental pushes omit
    /// unchanged fields (e.g. a recolor sends only `tray_color`; a
    /// mid-transition push sends only `{"id"}`), so the merge faithfully
    /// tracks the printer's per-field state without emptiness
    /// heuristics.
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
            err_code,
            fan_speed,
        );
        // AMS: spool-aware per-tray merge.
        if let Some(patch_ams) = patch.ams {
            match &mut self.ams {
                None => self.ams = Some(patch_ams),
                Some(cached) => cached.merge_in(patch_ams),
            }
        }
        // vt_tray: field-level merge. BBL sends partial external-spool
        // pushes carrying a single changed field (recolor on the
        // printer → `{tray_color}` only), so merging per-field rather
        // than replacing wholesale keeps the cached tray_type/id intact.
        // Placeholder fields are skipped inside `merge_in`.
        if let Some(patch_vt) = patch.vt_tray {
            self.vt_tray
                .get_or_insert_with(RawAmsTray::default)
                .merge_in(patch_vt);
        }
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
    serde_json::from_slice(bytes).map_err(|e| format!("bambu report parse: {e}"))
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
        snapshot.job.get_or_insert(default_job()).file_name = Some(file);
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
        snapshot.job.get_or_insert(default_job()).eta_seconds = Some((m * 60.0) as u64);
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
        // Non-zero err_code = the printer rejected our command; sticky
        // until a later 0 (success) clears it. The frontend surfaces it.
        if let Some(e) = msg.err_code {
            extra.command_error_code = if e == 0 { None } else { Some(e as i32) };
        }
        if let Some(f) = msg.fan_speed {
            extra.fan_speed = Some(f as f32);
        }
        // External spool: `vt_tray` carries the slot's *remembered*
        // filament identity, which the printer keeps reporting even
        // after the spool is unloaded — so its presence does NOT mean
        // "loaded". The engaged signal is the active-tray pointer:
        // `tray_now == 254` is the external-spool tray id (AMS trays
        // are 0..N, 255 is "nothing engaged"). Gate the loadout on
        // that so an unloaded external reads empty, not stale. The
        // identity itself stays cached across load/unload (the printer
        // keeps reporting it), so no refresh request is needed — the
        // pointer alone decides whether to surface it.
        // Read before the AMS block consumes `msg.ams`.
        let external_engaged =
            msg.ams.as_ref().and_then(|a| a.tray_now) == Some(EXTERNAL_SPOOL_TRAY_ID);
        extra.external_spool = external_engaged
            .then(|| msg.vt_tray.as_ref().and_then(tray_identity))
            .flatten();
        // AMS: lower the (already placeholder-gated) raw AMS into the
        // typed shape. The caller is expected to feed a report whose
        // AMS was accumulated through `BambuReport::merge` (the live
        // `run_worker` does), so the spool-aware gate has already
        // dropped BBL's startup placeholder/empty pushes; here we just
        // convert. Replacing wholesale is correct because the gated
        // `raw` is the full cached state, not a raw per-message delta.
        if let Some(raw) = msg.ams {
            extra.ams = Some(raw.to_typed());
        }
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
        "IDLE" => JobState::Idle,
        "PREPARE" => JobState::Preparing,
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
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(1000));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Snapshot we accumulate deltas into between ticks.
    let mut pending: Option<PrinterStatus> = None;
    let mut dirty = false;
    // Raw report accumulator. Each delta is merged through
    // `BambuReport::merge` so the spool-aware AMS gate
    // (`RawAmsState::merge_in`) drops BBL's placeholder/empty pushes
    // during print startup instead of letting them clobber the cached
    // AMS state. We then lower the accumulated report into the typed
    // snapshot, so `extra.ams` only ever reflects gated, real state.
    let mut acc = BambuReport::default();

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
                        // Accumulate through the gated merge, then
                        // lower the full accumulated report. Scalars
                        // are last-write-wins (re-applying is
                        // idempotent); AMS is placeholder-gated.
                        acc.merge(msg.print);
                        merge_into(snapshot, acc.clone());
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
        }))
        .unwrap();
        assert_eq!(msg_num.print.layer_num, Some(42));

        let msg_str: BambuMessage = serde_json::from_value(json!({
            "print": { "layer_num": "42" }
        }))
        .unwrap();
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
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
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
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
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
    fn merge_surfaces_command_err_code_and_clears_on_zero() {
        // 84033543 (Developer Mode off) surfaces as command_error_code;
        // a later 0 clears it. Goes through the worker path (acc.merge →
        // merge_into) so a field missing from `lww!` (the real bug) is
        // caught.
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        let mut acc = BambuReport::default();

        acc.merge(BambuReport {
            err_code: Some(84033543),
            ..Default::default()
        });
        merge_into(&mut snap, acc.clone());
        let DriverExtra::Bambu(e) = &snap.extra else {
            panic!("bambu")
        };
        assert_eq!(e.command_error_code, Some(84033543));

        acc.merge(BambuReport {
            err_code: Some(0),
            ..Default::default()
        });
        merge_into(&mut snap, acc.clone());
        let DriverExtra::Bambu(e) = &snap.extra else {
            panic!("bambu")
        };
        assert_eq!(e.command_error_code, None);
    }

    #[test]
    fn merge_into_advances_last_updated() {
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        let before = snap.last_updated;
        std::thread::sleep(std::time::Duration::from_millis(5));
        merge_into(&mut snap, BambuReport::default());
        assert!(
            snap.last_updated > before,
            "last_updated must advance on each merge"
        );
    }

    #[test]
    fn parse_ams_4_loaded_typed() {
        // Captured-shape fixture: 4 trays each loaded with a
        // distinct PLA color. Active = slot 2 (tray_now encoding
        // = unit 0 * 4 + tray 2).
        let msg: BambuMessage = serde_json::from_value(json!({
            "print": {
                "ams": {
                    "tray_now": "2",
                    "ams": [{
                        "id": "0",
                        "tray": [
                            {"id": "0", "tray_type": "PLA", "tray_color": "FF0000FF"},
                            {"id": "1", "tray_type": "PLA", "tray_color": "00FF00FF"},
                            {"id": "2", "tray_type": "PLA", "tray_color": "0000FFFF"},
                            {"id": "3", "tray_type": "PETG", "tray_color": "FFFF00FF"}
                        ]
                    }]
                }
            }
        }))
        .unwrap();
        let raw = msg.print.ams.expect("ams present");
        let typed = raw.to_typed();
        assert_eq!(typed.active_slot, Some(2));
        assert_eq!(typed.units.len(), 1);
        assert_eq!(typed.units[0].trays.len(), 4);
        for (i, t) in typed.units[0].trays.iter().enumerate() {
            let id = t.identity.as_ref().expect("loaded");
            assert!(!id.color.is_empty(), "tray {i} color");
            assert!(!id.tray_type.is_empty(), "tray {i} type");
        }
    }

    #[test]
    fn parse_ams_3_loaded_1_empty() {
        // Slot 3 reports tray_color="00000000" — a Bambu
        // "empty" sentinel that the normalizer must not
        // surface as a phantom black spool.
        let msg: BambuMessage = serde_json::from_value(json!({
            "print": {
                "ams": {
                    "tray_now": "0",
                    "ams": [{
                        "id": 0,
                        "tray": [
                            {"id": 0, "tray_type": "PLA", "tray_color": "FF0000FF"},
                            {"id": 1, "tray_type": "PLA", "tray_color": "00FF00FF"},
                            {"id": 2, "tray_type": "PETG", "tray_color": "0000FFFF"},
                            {"id": 3, "tray_color": "00000000"}
                        ]
                    }]
                }
            }
        }))
        .unwrap();
        let typed = msg.print.ams.unwrap().to_typed();
        assert!(typed.units[0].trays[0].identity.is_some());
        assert!(typed.units[0].trays[1].identity.is_some());
        assert!(typed.units[0].trays[2].identity.is_some());
        assert!(
            typed.units[0].trays[3].identity.is_none(),
            "empty tray (00000000) must surface as None, not a black spool"
        );
    }

    #[test]
    fn parse_ams_multicolor_spool() {
        let msg: BambuMessage = serde_json::from_value(json!({
            "print": {
                "ams": {
                    "ams": [{
                        "id": 0,
                        "tray": [{
                            "id": 0,
                            "tray_type": "PLA",
                            "tray_color": "FF0000FF",
                            "cols": ["FF0000FF", "00FF00FF", "0000FFFF"]
                        }]
                    }]
                }
            }
        }))
        .unwrap();
        let typed = msg.print.ams.unwrap().to_typed();
        let id = typed.units[0].trays[0].identity.as_ref().unwrap();
        assert_eq!(id.multi_colors.len(), 3);
        assert_eq!(id.color, "FF0000FF");
    }

    #[test]
    fn parse_ams_active_slot_decodes_multi_unit_encoding() {
        // tray_now = 5 in a 2-unit system encodes unit 1, tray 1
        // (1 * 4 + 1 = 5). We surface a flat slot index of 5.
        let msg: BambuMessage = serde_json::from_value(json!({
            "print": {
                "ams": {
                    "tray_now": "5",
                    "ams": [
                        {"id": 0, "tray": []},
                        {"id": 1, "tray": []}
                    ]
                }
            }
        }))
        .unwrap();
        let typed = msg.print.ams.unwrap().to_typed();
        assert_eq!(typed.active_slot, Some(5));
    }

    #[test]
    fn merge_into_populates_typed_ams_under_bambu_extra() {
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        let report: BambuReport = serde_json::from_value(json!({
            "ams": {
                "tray_now": 0,
                "ams": [{
                    "id": 0,
                    "tray": [
                        {"id": 0, "tray_type": "PLA", "tray_color": "FF8800FF"}
                    ]
                }]
            }
        }))
        .unwrap();
        merge_into(&mut snap, report);
        match snap.extra {
            DriverExtra::Bambu(extra) => {
                let ams = extra.ams.expect("ams populated");
                assert_eq!(ams.active_slot, Some(0));
                assert_eq!(
                    ams.units[0].trays[0].identity.as_ref().unwrap().color,
                    "FF8800FF"
                );
            }
            _ => panic!("expected Bambu extra"),
        }
    }

    #[test]
    fn external_spool_surfaces_only_when_engaged() {
        // vt_tray carries a full identity (the printer keeps reporting
        // the slot's remembered filament even when unloaded). With the
        // external NOT engaged (tray_now = 255), the loadout must read
        // empty, not show the stale identity.
        let unengaged: BambuReport = serde_json::from_value(json!({
            "ams": { "tray_now": 255, "ams": [] },
            "vt_tray": { "id": 254, "tray_type": "PLA", "tray_color": "101410FF" },
        }))
        .unwrap();
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        merge_into(&mut snap, unengaged);
        match &snap.extra {
            DriverExtra::Bambu(extra) => assert!(
                extra.external_spool.is_none(),
                "unengaged external must not surface its remembered identity"
            ),
            _ => panic!("expected Bambu extra"),
        }

        // Same identity, now engaged (tray_now = 254) → it surfaces.
        let engaged: BambuReport = serde_json::from_value(json!({
            "ams": { "tray_now": 254, "ams": [] },
            "vt_tray": { "id": 254, "tray_type": "PLA", "tray_color": "101410FF" },
        }))
        .unwrap();
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        merge_into(&mut snap, engaged);
        match &snap.extra {
            DriverExtra::Bambu(extra) => {
                let ext = extra
                    .external_spool
                    .as_ref()
                    .expect("engaged external surfaces");
                assert_eq!(ext.color, "101410FF");
            }
            _ => panic!("expected Bambu extra"),
        }
    }

    #[test]
    fn partial_vt_tray_push_patches_one_field_keeps_the_rest() {
        // Full external-spool identity from a pushall, then a partial
        // incremental push that only recolors it (what the printer
        // sends when you change the spool's color in its UI). The new
        // color must apply while tray_type / sku survive.
        let mut acc = BambuReport::default();
        acc.merge(
            serde_json::from_value::<BambuReport>(json!({
                "vt_tray": {
                    "id": 254, "tray_type": "PLA",
                    "tray_color": "101410FF", "tray_info_idx": "GFL99",
                },
            }))
            .unwrap(),
        );
        acc.merge(
            serde_json::from_value::<BambuReport>(json!({
                "vt_tray": { "tray_color": "AC95D5FF" },
            }))
            .unwrap(),
        );
        let vt = acc.vt_tray.expect("vt_tray retained");
        assert_eq!(vt.color.as_deref(), Some("AC95D5FF"), "recolor applied");
        assert_eq!(vt.material.as_deref(), Some("PLA"), "tray_type survives");
        assert_eq!(vt.tray_info_idx.as_deref(), Some("GFL99"), "sku survives");
        assert_eq!(vt.id, Some(254), "id survives");
    }

    // ---- spool-aware AMS merge (the print-startup data-loss fix) ----

    /// Helper: a real-spool tray. `cols: None` = the field was omitted
    /// (not "present empty").
    fn real_tray(id: i64, material: &str, color: &str) -> RawAmsTray {
        RawAmsTray {
            id: Some(id),
            material: Some(material.into()),
            color: Some(color.into()),
            sub_brand: None,
            cols: None,
            tray_info_idx: None,
            tag_uid: None,
        }
    }

    /// A partial / mid-transition push: only `id` is present, every
    /// other field omitted — exactly what the A1 mini sends as
    /// `{"id":"0"}` between a full state and the next (verified on the
    /// wire). All non-id fields are `None` (absent), so the merge must
    /// leave the cached tray untouched.
    fn placeholder_tray(id: i64) -> RawAmsTray {
        RawAmsTray {
            id: Some(id),
            ..Default::default()
        }
    }

    #[test]
    fn tray_merge_is_presence_based_absent_keeps_present_overwrites() {
        // Absent fields (a partial push) leave the cached tray intact.
        let mut cached = real_tray(0, "PLA", "FF8800FF");
        cached.merge_in(placeholder_tray(0));
        assert_eq!(cached.material.as_deref(), Some("PLA"));
        assert_eq!(cached.color.as_deref(), Some("FF8800FF"));

        // A present field overwrites — built up one incremental push at
        // a time, each carrying only its changed field.
        let mut t = RawAmsTray::default();
        t.merge_in(RawAmsTray {
            color: Some("FF0000FF".into()),
            ..Default::default()
        });
        t.merge_in(RawAmsTray {
            sub_brand: Some("Bambu PLA Basic".into()),
            ..Default::default()
        });
        t.merge_in(RawAmsTray {
            cols: Some(vec!["00FF00FF".into()]),
            ..Default::default()
        });
        assert_eq!(t.color.as_deref(), Some("FF0000FF"));
        assert_eq!(t.sub_brand.as_deref(), Some("Bambu PLA Basic"));
        assert_eq!(t.cols, Some(vec!["00FF00FF".to_string()]));

        // A present *empty* value is a real state change and overwrites
        // (clears) — the bug this rewrite fixes. A spool swap that drops
        // the RFID tag / sub-brand must not leave the stale value cached.
        t.merge_in(RawAmsTray {
            sub_brand: Some(String::new()),
            tag_uid: Some(String::new()),
            ..Default::default()
        });
        assert_eq!(t.sub_brand.as_deref(), Some(""), "present empty clears");
        assert_eq!(t.tag_uid.as_deref(), Some(""), "present empty clears");
        // ...but a field the same patch omitted is untouched.
        assert_eq!(t.color.as_deref(), Some("FF0000FF"), "omitted color kept");
    }

    #[test]
    fn tray_merge_distinguishes_omitted_from_empty_at_the_serde_boundary() {
        // The distinction only exists in the wire JSON — prove it
        // round-trips through deserialization, not just typed literals.
        let mut cached: RawAmsTray =
            serde_json::from_str(r#"{"id":"0","tray_info_idx":"GFA00","tag_uid":"BC6CF90100000100"}"#)
                .unwrap();

        // A `{"id":"0"}` partial push omits tray_info_idx/tag_uid → kept.
        let partial: RawAmsTray = serde_json::from_str(r#"{"id":"0"}"#).unwrap();
        cached.merge_in(partial);
        assert_eq!(cached.tray_info_idx.as_deref(), Some("GFA00"));
        assert_eq!(cached.tag_uid.as_deref(), Some("BC6CF90100000100"));

        // A swap to a non-RFID generic: the printer sends a real new
        // tray_info_idx and the all-zeros tag — both present → adopted.
        let swap: RawAmsTray =
            serde_json::from_str(r#"{"id":"0","tray_info_idx":"GFL99","tag_uid":"0000000000000000"}"#)
                .unwrap();
        cached.merge_in(swap);
        assert_eq!(cached.tray_info_idx.as_deref(), Some("GFL99"));
        assert_eq!(cached.tag_uid.as_deref(), Some("0000000000000000"));
        assert!(!crate::core::driver::status::rfid_detected(
            cached.tag_uid.as_deref()
        ));
    }

    #[test]
    fn merge_keeps_cached_real_data_when_patch_is_all_placeholders() {
        // The bug we're fixing: initial pushall carried 4 real
        // trays; a follow-up startup push carried 4 placeholders;
        // the wholesale-replace merge wiped the cached identities
        // and the panel showed empty AMS slots mid-print.
        let mut cached = RawAmsState {
            tray_now: Some(0),
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![
                    real_tray(0, "PLA", "FF0000FF"),
                    real_tray(1, "PETG", "00FF00FF"),
                    real_tray(2, "PLA", "0000FFFF"),
                    real_tray(3, "ABS", "FFFF00FF"),
                ],
            }],
        };
        let patch = RawAmsState {
            tray_now: Some(1),
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: (0..4).map(placeholder_tray).collect(),
            }],
        };
        cached.merge_in(patch);
        // tray_now advanced.
        assert_eq!(cached.tray_now, Some(1));
        // But the trays kept their cached identities.
        assert_eq!(cached.ams[0].tray[0].material.as_deref(), Some("PLA"));
        assert_eq!(cached.ams[0].tray[0].color.as_deref(), Some("FF0000FF"));
        assert_eq!(cached.ams[0].tray[2].material.as_deref(), Some("PLA"));
        assert_eq!(cached.ams[0].tray[3].material.as_deref(), Some("ABS"));
    }

    #[test]
    fn worker_accumulation_keeps_typed_ams_through_placeholder_push() {
        // Mirrors run_worker's pipeline: accumulate raw reports via
        // BambuReport::merge, then lower the accumulated report via
        // merge_into. A placeholder push after a real pushall must not
        // wipe the TYPED extra.ams (regression: the worker used to call
        // merge_into per-message, bypassing the gate, so a startup
        // placeholder cleared the AMS state the sync/UI read).
        let mut acc = BambuReport::default();
        acc.merge(BambuReport {
            ams: Some(RawAmsState {
                tray_now: Some(0),
                ams: vec![RawAmsUnit {
                    id: Some(0),
                    tray: vec![real_tray(0, "PLA", "FF0000FF")],
                }],
            }),
            ..Default::default()
        });
        acc.merge(BambuReport {
            ams: Some(RawAmsState {
                tray_now: Some(0),
                ams: vec![RawAmsUnit {
                    id: Some(0),
                    tray: vec![placeholder_tray(0)],
                }],
            }),
            ..Default::default()
        });
        let mut snap = PrinterStatus::disconnected_for(DriverExtra::Bambu(BambuExtra::default()));
        merge_into(&mut snap, acc.clone());
        let DriverExtra::Bambu(extra) = &snap.extra else {
            panic!("expected Bambu extra");
        };
        let ams = extra.ams.as_ref().expect("ams present after merge");
        assert_eq!(ams.units.len(), 1, "unit count preserved");
        assert_eq!(ams.units[0].trays.len(), 1);
        assert!(
            ams.units[0].trays[0].identity.is_some(),
            "real spool identity survived the placeholder push",
        );
    }

    #[test]
    fn merge_adopts_real_patch_trays_and_keeps_placeholder_positions() {
        // Mixed patch: two real, two placeholder. The real ones
        // overwrite cached values at those positions; the placeholder
        // positions keep the cached tray.
        let mut cached = RawAmsState {
            tray_now: None,
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![
                    real_tray(0, "PLA", "FF0000FF"),
                    real_tray(1, "PLA", "00FF00FF"),
                    real_tray(2, "PLA", "0000FFFF"),
                    real_tray(3, "PLA", "FFFF00FF"),
                ],
            }],
        };
        let patch = RawAmsState {
            tray_now: None,
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![
                    real_tray(0, "PETG", "FF00FFFF"), // updates [0]
                    placeholder_tray(1),              // preserve cached [1]
                    placeholder_tray(2),              // preserve cached [2]
                    real_tray(3, "ABS", "00FFFFFF"),  // updates [3]
                ],
            }],
        };
        cached.merge_in(patch);
        assert_eq!(cached.ams[0].tray[0].material.as_deref(), Some("PETG"));
        assert_eq!(cached.ams[0].tray[0].color.as_deref(), Some("FF00FFFF"));
        assert_eq!(cached.ams[0].tray[1].material.as_deref(), Some("PLA")); // cached
        assert_eq!(cached.ams[0].tray[1].color.as_deref(), Some("00FF00FF")); // cached
        assert_eq!(cached.ams[0].tray[2].color.as_deref(), Some("0000FFFF")); // cached
        assert_eq!(cached.ams[0].tray[3].material.as_deref(), Some("ABS"));
    }

    #[test]
    fn merge_accepts_patch_verbatim_when_no_cached_ams() {
        // First sighting — accept the patch as-is even if it carries
        // placeholders. A later real-data patch will refine.
        let mut report = BambuReport::default();
        let patch = BambuReport {
            ams: Some(RawAmsState {
                tray_now: Some(2),
                ams: vec![RawAmsUnit {
                    id: Some(0),
                    tray: vec![placeholder_tray(0), placeholder_tray(1)],
                }],
            }),
            ..Default::default()
        };
        report.merge(patch);
        let ams = report.ams.expect("ams populated");
        assert_eq!(ams.tray_now, Some(2));
        assert_eq!(ams.ams[0].tray.len(), 2);
    }

    #[test]
    fn merge_updates_tray_now_even_when_unit_data_is_placeholder() {
        // Active-slot advancement is the one signal we trust from a
        // placeholder push — `tray_now` is a scalar that BBL keeps
        // accurate even when it doesn't bother re-sending the tray
        // identities. Without this, the panel's active-slot ring
        // would never move during a print.
        let mut cached = RawAmsState {
            tray_now: Some(0),
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![real_tray(0, "PLA", "FF0000FF")],
            }],
        };
        let patch = RawAmsState {
            tray_now: Some(3),
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![placeholder_tray(0)],
            }],
        };
        cached.merge_in(patch);
        assert_eq!(cached.tray_now, Some(3));
        assert_eq!(cached.ams[0].tray[0].material.as_deref(), Some("PLA"));
    }

    #[test]
    fn merge_appends_new_units_beyond_cached_length() {
        // X1C with multiple AMS units can grow the unit list after
        // initial connection. New units arriving in a patch should
        // be appended; if all their trays are placeholders the
        // append still happens (so the *positions* are reserved) but
        // the trays carry no spool identity yet — a later real-data
        // patch will fill them.
        let mut cached = RawAmsState {
            tray_now: None,
            ams: vec![RawAmsUnit {
                id: Some(0),
                tray: vec![real_tray(0, "PLA", "FF0000FF")],
            }],
        };
        let patch = RawAmsState {
            tray_now: None,
            ams: vec![
                RawAmsUnit {
                    id: Some(0),
                    tray: vec![placeholder_tray(0)],
                },
                RawAmsUnit {
                    id: Some(1),
                    tray: vec![real_tray(0, "PETG", "00FF00FF")],
                },
            ],
        };
        cached.merge_in(patch);
        assert_eq!(cached.ams.len(), 2);
        // Cached unit's real tray preserved.
        assert_eq!(cached.ams[0].tray[0].material.as_deref(), Some("PLA"));
        // New unit's real tray appended.
        assert_eq!(cached.ams[1].tray[0].material.as_deref(), Some("PETG"));
    }

    #[test]
    fn is_transparent_black_classifies_known_sentinels() {
        assert!(is_transparent_black("00000000"));
        assert!(is_transparent_black("000000")); // 6-hex form
        assert!(is_transparent_black("#000000")); // with hash
        assert!(!is_transparent_black("FF0000FF"));
        assert!(!is_transparent_black("000001FF")); // not exactly 000000
        assert!(!is_transparent_black("000000FF")); // alpha != 00
    }

    #[test]
    fn state_mapping_covers_known_strings() {
        assert!(matches!(map_state("IDLE"), JobState::Idle));
        assert!(matches!(map_state("PREPARE"), JobState::Preparing));
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
