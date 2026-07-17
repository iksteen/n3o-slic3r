//! Tauri command surface for the driver layer.
//!
//! Eight commands cover the registry + per-driver lifecycle.
//! `driver_register` is the only driver-kind-aware one — it
//! takes a [`DriverConfig`] variant and instantiates the right
//! `Driver` impl (Bambu or U1).
//!
//! Status updates emit on `driver:status_update` as a Tauri
//! event with payload `{ driver_id, status }`. Driver workers
//! hook the event emission into their rate-limited
//! `watch::Sender<PrinterStatus>` pipelines.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use super::ams::css_to_hex8;
use super::bambu::connection::{BambuConfig, BambuDriver};
use super::moonraker::{MoonrakerConfig, MoonrakerDriver, StatusSessionFactory, WsSessionFactory};
use super::registry::DriverRegistry;
use super::snapmaker::{mqtt_status, snap_token};
use super::send::{
    apply_pre_send, collect_ams_bindings, collect_ams_mapping, derive_send_names,
    logical_nozzle_diameters, physical_extruders_used, plate_nozzle_diameters,
    plate_printer_model, plate_send_options, read_gcode_bytes, u1_map_table,
    u1_usage_from_gcode, wrap_gcode_as_3mf,
};
use super::status::PrinterStatus;
use super::traits::{
    Driver, DriverConfig, DriverError, DriverId, DriverKind, PrinterCommand, SendHandle,
    SendPayload, U1StartOptions, UploadProgressFn,
};
use crate::core::plugin::commands::PluginHostState;
use crate::core::project::Project;

/// Wire-shape for the `driver:status_update` Tauri event the
/// frontend's `useDriverStatus` hook subscribes to. Carries the
/// driver id so the hook can filter to just the panel's driver.
#[derive(Debug, Clone, Serialize)]
struct StatusUpdateEvent {
    driver_id: DriverId,
    status: PrinterStatus,
}

/// Wire-shape for the `driver:upload_progress` Tauri event the frontend's
/// `useUploadProgress` hook subscribes to. Emitted (throttled) while a send is
/// pushing the bundle to the printer; the hook filters on `driver_id`.
#[derive(Debug, Clone, Serialize)]
struct UploadProgress {
    driver_id: DriverId,
    file_name: String,
    sent: u64,
    total: u64,
    percent: u8,
}

/// Per-driver cancellation tokens for in-flight sends. `driver_send_plate`
/// arms a token per send; `driver_send_cancel` fires it, and the
/// `tokio::select!` in `driver_send_plate` then drops the in-flight `send`
/// future to abort the upload. `send()` only takes a shared read lock, so this
/// registry — not the lock — is what enforces one upload per driver: `arm`
/// refuses a second concurrent send for the same id, and the returned
/// [`SendGuard`] disarms the slot on drop (including panic unwind).
/// Tauri-managed; `.manage(...)` it from `lib.rs`.
#[derive(Default)]
pub struct SendCancelRegistry {
    tokens: Mutex<HashMap<DriverId, CancellationToken>>,
}

/// RAII arm for one in-flight send. Removes the driver's token on drop, so a
/// settled (or panicked) send frees the slot for the next one.
pub struct SendGuard {
    registry: Arc<SendCancelRegistry>,
    id: DriverId,
    token: CancellationToken,
}

impl SendGuard {
    /// The send's cancel token, to race in `select!`.
    fn token(&self) -> &CancellationToken {
        &self.token
    }
}

impl Drop for SendGuard {
    fn drop(&mut self) {
        self.registry.tokens.lock().unwrap().remove(&self.id);
    }
}

impl SendCancelRegistry {
    /// Arm a cancel token for `id`, or `None` if a send is already in flight for
    /// that driver. The returned guard disarms the slot on drop.
    fn arm(self: Arc<Self>, id: DriverId) -> Option<SendGuard> {
        let token = {
            let mut tokens = self.tokens.lock().unwrap();
            if tokens.contains_key(&id) {
                return None;
            }
            let token = CancellationToken::new();
            tokens.insert(id, token.clone());
            token
        };
        Some(SendGuard { registry: self, id, token })
    }
    /// Fire `id`'s token if a send is in flight; a no-op otherwise.
    fn cancel(&self, id: DriverId) {
        if let Some(token) = self.tokens.lock().unwrap().get(&id) {
            token.cancel();
        }
    }
}

/// Build the [`UploadProgressFn`] a driver's `send` calls as bytes go out. It
/// derives a percent and emits `driver:upload_progress`, throttled to ~one event
/// per 50 ms (plus a guaranteed final 100%) — the same cadence as
/// `SliceEvent::PlateProgress`, so a fast LAN upload doesn't flood the channel.
fn upload_progress_emitter(
    app: AppHandle,
    driver_id: DriverId,
    file_name: String,
) -> UploadProgressFn {
    use std::sync::atomic::{AtomicU64, Ordering};
    let last_ms = Arc::new(AtomicU64::new(0));
    Arc::new(move |sent: u64, total: u64| {
        let percent = if total > 0 {
            ((sent.min(total) * 100) / total) as u8
        } else {
            0
        };
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let prev = last_ms.load(Ordering::Relaxed);
        if percent >= 100 || now_ms.saturating_sub(prev) >= 50 {
            last_ms.store(now_ms, Ordering::Relaxed);
            let _ = app.emit(
                "driver:upload_progress",
                UploadProgress {
                    driver_id,
                    file_name: file_name.clone(),
                    sent,
                    total,
                    percent,
                },
            );
        }
    })
}

/// Spawn a tokio task that pumps a driver's internal
/// `watch::Receiver<PrinterStatus>` to a Tauri event. Lives for
/// the driver's lifetime — the watch channel closes when the
/// driver is dropped (driver_unregister + registry remove),
/// which ends the task naturally. Per-driver rate-limiting
/// happens in the driver's own worker; this bridge just forwards
/// every change without filtering.
fn spawn_status_bridge(
    app: AppHandle,
    driver_id: DriverId,
    mut rx: tokio::sync::watch::Receiver<PrinterStatus>,
) {
    tauri::async_runtime::spawn(async move {
        // Emit the initial state once on spawn — `changed()` only
        // fires on subsequent writes, so without this the panel
        // would show a stale empty state until the first reconnect
        // or status report.
        let initial = rx.borrow().clone();
        let _ = app.emit(
            "driver:status_update",
            StatusUpdateEvent {
                driver_id,
                status: initial,
            },
        );
        while rx.changed().await.is_ok() {
            let status = rx.borrow().clone();
            if app
                .emit(
                    "driver:status_update",
                    StatusUpdateEvent { driver_id, status },
                )
                .is_err()
            {
                // App shutting down; stop the bridge.
                break;
            }
        }
    });
}

/// Register a fresh driver instance with the runtime. Returns
/// the allocated [`DriverId`] on success. Doesn't auto-connect
/// — caller follows up with [`driver_connect`].
///
/// As a side-effect, spawns a status-bridge task that pumps the
/// driver's `subscribe_status` channel onto the
/// `driver:status_update` Tauri event so the frontend's
/// `useDriverStatus` hook can react without polling.
/// Construct the concrete driver for a [`DriverConfig`] variant.
/// Shared by [`driver_register`] (which inserts it into the registry +
/// spawns the status bridge) and [`driver_test_connection`] (which
/// drives a throwaway instance and discards it), so the per-kind
/// construction lives in one place.
fn build_driver(id: DriverId, instance_id: &str, config: DriverConfig) -> Box<dyn Driver> {
    match config {
        DriverConfig::Bambu { host, access_code } => {
            Box::new(BambuDriver::new(id, BambuConfig { host, access_code }))
        }
        // Both Moonraker-backed kinds run the same driver; the kind only
        // distinguishes the vendor webcam stack (see `camera::source_for`).
        // A paired U1 carries an mTLS token → status rides the vendor MQTT
        // bus (remote-capable); unpaired/generic → the open WebSocket. The
        // driver stays vendor-agnostic; we inject the transport here.
        DriverConfig::U1 { host, port } => {
            let factory: Arc<dyn StatusSessionFactory> = match snap_token::load(instance_id) {
                Some(token) => Arc::new(mqtt_status::MqttSessionFactory { token }),
                None => Arc::new(WsSessionFactory {
                    host: host.clone(),
                    port,
                }),
            };
            Box::new(MoonrakerDriver::new(
                id,
                DriverKind::U1,
                MoonrakerConfig { host, port },
                factory,
            ))
        }
        DriverConfig::Moonraker { host, port } => Box::new(MoonrakerDriver::new(
            id,
            DriverKind::Moonraker,
            MoonrakerConfig {
                host: host.clone(),
                port,
            },
            Arc::new(WsSessionFactory { host, port }),
        )),
    }
}

#[tauri::command]
#[tracing::instrument(skip(registry, app))]
pub async fn driver_register(
    config: DriverConfig,
    instance_id: String,
    app: AppHandle,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<DriverId, String> {
    // `register_with` allocates the id atomically with insertion so the
    // driver's internal `id()` matches the registry's id (drivers use
    // it for log spans + outgoing protocol frames).
    let mut bridge_rx = None;
    let id = registry.register_with(&instance_id, |id| {
        let driver = build_driver(id, &instance_id, config);
        bridge_rx = Some(driver.subscribe_status());
        driver
    });
    if let Some(rx) = bridge_rx {
        spawn_status_bridge(app, id, rx);
    }
    Ok(id)
}

/// Test a connection config WITHOUT registering a driver or touching
/// the live registry. Builds a transient driver, connects, and waits
/// for the connection to reach `Connected` (→ `Ok`) or `Disconnected`
/// (→ `Err` with the printer's reason), tearing it down either way. A
/// timeout guards against a silently-unreachable printer hanging the
/// UI.
///
/// Backs the settings modal's "Test connection" button: the verdict is
/// returned synchronously, nothing is persisted, and the
/// auto-connection reconciler is not involved.
#[tauri::command]
#[tracing::instrument]
pub async fn driver_test_connection(
    config: DriverConfig,
    instance_id: String,
) -> Result<(), String> {
    use super::status::ConnectionState;

    // Generous cap covering Bambu's ~5-8s MQTT handshake; U1's HTTP
    // probe is faster.
    const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    // Transient driver. DriverId(0) is fine — it's never inserted into
    // the registry; the id only tags log spans / outgoing frames.
    let mut driver = build_driver(DriverId(0), &instance_id, config);

    // Subscribe before connecting and mark the initial pre-connect
    // state seen, so the watch loop only reacts to transitions the
    // connect actually drives (not the "not yet connected" baseline).
    let mut rx = driver.subscribe_status();
    let _ = rx.borrow_and_update();

    // A hard failure inside connect() (e.g. U1's system_info probe
    // against an unreachable host) surfaces immediately.
    driver.connect().await.map_err(|e| e.to_string())?;

    let verdict = tokio::time::timeout(TEST_TIMEOUT, async {
        loop {
            if rx.changed().await.is_err() {
                return Err("driver stopped before reporting a connection".to_string());
            }
            match &rx.borrow_and_update().connection {
                ConnectionState::Connected => return Ok(()),
                ConnectionState::Disconnected { reason } => return Err(reason.clone()),
                // On a FRESH connect the path is Connecting → Connected
                // (success) or Connecting → Reconnecting (the attempt
                // failed and the driver is backing off). For a one-shot
                // test there's nothing to wait for once it's retrying —
                // report the failure reason the driver threaded into
                // Reconnecting rather than hanging to the 15s timeout.
                // (A wrong Bambu access code lands here, not in
                // Disconnected.)
                ConnectionState::Reconnecting { reason, .. } => {
                    return Err(reason.clone());
                }
                // Connecting — keep waiting for the verdict.
                ConnectionState::Connecting => {}
            }
        }
    })
    .await;

    // Always tear the transient driver down, whatever the verdict.
    let _ = driver.disconnect().await;

    match verdict {
        Ok(inner) => inner,
        Err(_elapsed) => Err("Timed out waiting for the printer to connect".to_string()),
    }
}

/// Tear down + remove a driver. Calls `disconnect()` first.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_unregister(
    id: DriverId,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    if let Some(handle) = registry.get(id) {
        let mut d = handle.write().await;
        // Best-effort disconnect; we remove regardless of result.
        let _ = d.disconnect().await;
    }
    registry.remove(id);
    Ok(())
}

/// Open the printer connection. Spawns the driver's background
/// task (rumqttc loop / Moonraker WebSocket worker). Idempotent.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_connect(
    id: DriverId,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.write().await;
    d.connect().await.map_err(|e| e.to_string())
}

/// Tear down the connection cleanly. Idempotent.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_disconnect(
    id: DriverId,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.write().await;
    d.disconnect().await.map_err(|e| e.to_string())
}

/// Latest cached status snapshot for the driver. Cheap — reads
/// the driver's internal `watch` channel without contacting the
/// printer.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_status(
    id: DriverId,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<PrinterStatus, String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let d = handle.read().await;
    Ok(d.status())
}

/// Wrap a plate's last-sliced raw G-code into the same
/// `.gcode.3mf` bundle the driver send path uses, but write it
/// to disk instead of uploading. Diagnostic surface — lets the
/// user grab the exact bytes our send path produces so they can
/// diff vs BBS / other slicer outputs without fishing the bundle
/// out of the printer's /cache/ directory.
#[tauri::command]
#[tracing::instrument(skip(project))]
pub async fn driver_export_plate(
    plate_id: u32,
    gcode_path: String,
    output_path: String,
    thumbnail_png_base64: Option<String>,
    project: State<'_, Arc<Mutex<Project>>>,
) -> Result<(), String> {
    // MQTT mapping isn't surfaced in the exported bundle, but pull it
    // anyway so the .gcode.3mf side stays consistent with what the
    // send path would emit.
    let ams = collect_ams_bindings(&project, plate_id);
    let (_basename, title) = derive_send_names(&project, plate_id);
    let thumbnail_png = thumbnail_png_base64.and_then(|b64| {
        base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .ok()
    });
    let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, title, ams, thumbnail_png).await?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&output_path, &bytes).map_err(|e| format!("write {output_path}: {e}"))
    })
    .await
    .map_err(|e| format!("export task join: {e}"))?
}

/// Send the plate's last-sliced raw G-code to the driver. The
/// frontend obtains `gcode_path` from the most recent
/// `slice:plate_finished` event (the `output_path` field on
/// `PlateSummary`).
///
/// Payload shape depends on the driver kind:
/// - **Bambu** — wrap as `.gcode.3mf` via [`wrap_gcode_as_3mf`] and
///   ship as [`SendPayload::Gcode3mf`] with the plate's AMS routing.
///   The bundle embeds the raw G-code, the project/plate `Title`
///   metadata, the per-AMS slot bindings, and the plate thumbnail.
/// - **U1 / Moonraker** — ship the raw G-code body as
///   [`SendPayload::Gcode`]. Moonraker stores it under the supplied
///   file name and starts the print in the same multipart upload
///   (see `core/driver/moonraker/http.rs`).
#[tauri::command]
#[tracing::instrument(skip(registry, project, plugin_host, app, sends))]
pub async fn driver_send_plate(
    id: DriverId,
    plate_id: u32,
    gcode_path: String,
    app: AppHandle,
    // The active plate's preview, rendered by the viewport and passed as a
    // base64 PNG. `None` falls back to no thumbnail (both printers tolerate
    // its absence). Bambu embeds it in the `.gcode.3mf`; the U1 gets a
    // base64 comment block prepended to its raw G-code.
    thumbnail_png_base64: Option<String>,
    registry: State<'_, Arc<DriverRegistry>>,
    project: State<'_, Arc<Mutex<Project>>>,
    plugin_host: State<'_, PluginHostState>,
    sends: State<'_, Arc<SendCancelRegistry>>,
) -> Result<SendHandle, DriverError> {
    let handle = registry
        .get(id)
        .ok_or_else(|| DriverError::Other(format!("unknown driver id {}", id.0)))?;
    let kind = handle.read().await.kind();
    // Decode once; a malformed base64 is a soft failure — log and send
    // without a thumbnail rather than failing the print.
    let thumbnail_png: Option<Vec<u8>> = thumbnail_png_base64.and_then(|b64| {
        match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::warn!(error = %e, "thumbnail base64 decode failed; sending without it");
                None
            }
        }
    });
    let (basename, title) = derive_send_names(&project, plate_id);
    // The bound instance's sticky per-print toggles (the send dialog
    // edits them; the drivers translate to their wire fields).
    let options = plate_send_options(&project, plate_id);
    let payload = match kind {
        DriverKind::Bambu => {
            let ams = collect_ams_bindings(&project, plate_id);
            let (use_ams, ams_mapping, ams_mapping2) = collect_ams_mapping(&project, plate_id);
            let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, title, ams, thumbnail_png)
                .await
                .map_err(DriverError::Other)?;
            SendPayload::Gcode3mf {
                bytes,
                plate_id,
                file_basename: basename.clone(),
                use_ams,
                ams_mapping,
                ams_mapping2,
                options,
            }
        }
        DriverKind::U1 | DriverKind::Moonraker => {
            let bytes = read_gcode_bytes(gcode_path).await.map_err(DriverError::Other)?;
            // The U1 starts via its parameterized vendor macro, which
            // needs the print's per-extruder usage for the flow-cali
            // gate — read it off the G-code footer before the thumbnail
            // block goes in. Generic Moonraker has no option protocol.
            let u1_start = match kind {
                DriverKind::U1 => {
                    // Usage indices are logical (the G-code is now in
                    // logical material space); MAP_TABLE routes logical →
                    // physical at the printer. FILAMENT_USED_MM stays
                    // logical; FLOW_CALIBRATE_EXTRUDERS + NOZZLE_DIAMETER
                    // derive from the map table.
                    let (used_logical, filament_used_mm) = u1_usage_from_gcode(&bytes);
                    let map_table = u1_map_table(&project, plate_id);
                    let physical_nozzles = plate_nozzle_diameters(&project, plate_id);
                    Some(U1StartOptions {
                        options,
                        extruders_used: physical_extruders_used(&used_logical, &map_table),
                        filament_used_mm,
                        nozzle_diameters: logical_nozzle_diameters(&map_table, &physical_nozzles),
                        map_table,
                    })
                }
                _ => None,
            };
            // Prepend the Klipper/Moonraker thumbnail block so Mainsail /
            // Fluidd show the preview; a bad PNG leaves the G-code untouched.
            let bytes = match &thumbnail_png {
                Some(png) => super::thumbnail::prepend_thumbnail(bytes, png),
                None => bytes,
            };
            SendPayload::Gcode {
                bytes,
                file_name: format!("{basename}.gcode"),
                u1_start,
            }
        }
    };
    // Pre-send plugin hook: let plugins transform the bytes about to go
    // to the printer (sync — no await while the host lock is held).
    // Resolve the plate's printer model so the hook enforces
    // `printer_compatibility` (a plugin scoped to another model is
    // skipped even without a Lua self-guard).
    let printer_model = plate_printer_model(&project, plate_id);
    let payload = apply_pre_send(plugin_host.inner(), payload, plate_id, kind, printer_model);
    let file_name = match kind {
        DriverKind::Bambu => format!("{basename}.gcode.3mf"),
        DriverKind::U1 | DriverKind::Moonraker => format!("{basename}.gcode"),
    };
    let on_progress = upload_progress_emitter(app, id, file_name);
    // Arm a cancel token for this upload. `driver_send_cancel` fires it; the
    // select! then drops the in-flight `send` future, which aborts the upload
    // (every driver's `send` is fully async — Bambu's FTPS too). Rejects a
    // second concurrent send to the same driver — `send` takes only a shared
    // read lock, so nothing else serializes them. The guard disarms on drop.
    let cancel = match sends.inner().clone().arm(id) {
        Some(g) => g,
        None => {
            return Err(DriverError::Other(
                "a send is already in flight for this printer".into(),
            ))
        }
    };
    let d = handle.read().await;
    tokio::select! {
        r = d.send(payload, on_progress) => r,
        _ = cancel.token().cancelled() => Err(DriverError::Cancelled),
    }
}

/// Cancel an in-flight send to `id`. No-op when nothing is uploading. The
/// upload aborts with `DriverError::Cancelled`, which `driver_send_plate`
/// returns as an error the frontend recognizes (and treats as a user action,
/// not a failure).
#[tauri::command]
#[tracing::instrument(skip(sends))]
pub fn driver_send_cancel(
    id: DriverId,
    sends: State<'_, Arc<SendCancelRegistry>>,
) -> Result<(), String> {
    sends.cancel(id);
    Ok(())
}

/// Pause / resume / stop the current print.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_command(
    id: DriverId,
    cmd: PrinterCommand,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let d = handle.read().await;
    d.command(cmd).await.map_err(|e| e.to_string())
}

/// Push a UI-edited AMS slot's filament identity back to the printer
/// (Bambu AMS lite). Reads the slot's bound filament + color from the
/// instance, resolves the Bambu SKU + material + nozzle range from the
/// filament library, derives the `(ams_id, tray_id)` address from the
/// slot index, and dispatches `set_ams_filament`.
///
/// Refuses RFID-detected slots (printer-authoritative) and non-AMS
/// feeds. The frontend auto-fires this after a slot edit persists, but
/// only when a Bambu driver is connected — a disconnected driver makes
/// `set_ams_filament` return `NotConnected`, which the caller treats as
/// a non-fatal "edit saved locally, not pushed".
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_ams_set_filament(
    driver_id: DriverId,
    instance_id: String,
    extruder_idx: usize,
    slot_idx: usize,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    use crate::core::driver::status::{rfid_detected, JobState};
    use crate::core::driver::traits::AmsFilamentSetting;
    use crate::core::printer::instance::FeedKind;
    use crate::core::printer::instance_registry::lookup_instance;
    use crate::core::profile_library::{filament_nozzle_range, list_filament_fragments};

    let inst =
        lookup_instance(&instance_id).ok_or_else(|| format!("unknown instance {instance_id}"))?;
    let slot = inst
        .extruders
        .get(extruder_idx)
        .and_then(|e| e.slots.get(slot_idx))
        .ok_or_else(|| format!("slot {extruder_idx}/{slot_idx} out of range"))?;

    if !matches!(slot.feed, FeedKind::Ams) {
        return Err("slot is not an AMS feed".into());
    }
    if rfid_detected(slot.tag_uid.as_deref()) {
        return Err("slot is RFID-detected; managed by the printer".into());
    }
    let identity = slot
        .filament_identity
        .clone()
        .ok_or("slot has no filament bound")?;

    let library = list_filament_fragments();
    let frag = library
        .iter()
        .find(|f| f.identity == identity)
        .ok_or_else(|| format!("filament '{identity}' not in library"))?;
    let tray_info_idx = frag
        .filament_id
        .clone()
        .ok_or_else(|| format!("filament '{identity}' has no Bambu SKU"))?;
    let (nozzle_temp_min, nozzle_temp_max) =
        filament_nozzle_range(&identity).unwrap_or((frag.nozzle_temp, frag.nozzle_temp));
    let tray_color = slot
        .color
        .as_deref()
        .map(css_to_hex8)
        .unwrap_or_else(|| "FFFFFFFF".to_owned());

    // Reverse of `resolve_bambu`'s `unit_pos * 4 + tray.id`. For the A1
    // mini's regular AMS lite (ams_id <= 3) the in-unit slot_id equals
    // tray_id (BambuStudio sends both).
    let ams_id = (slot_idx / 4) as u8;
    let tray_id = (slot_idx % 4) as u8;

    let handle = registry
        .get(driver_id)
        .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
    let d = handle.read().await;
    // The printer rejects an AMS filament change while it's mid-print
    // (err 0x05024001 on the loaded tray). Don't send a doomed command —
    // the local binding already persisted; it'll push on the next edit
    // once the printer is idle. (Bambu gates the same op on RUNNING/PAUSE.)
    if matches!(
        d.status().job.as_ref().map(|j| &j.state),
        Some(JobState::Preparing | JobState::Printing | JobState::Paused)
    ) {
        return Err("printer is busy printing — AMS filament not updated on the device".into());
    }
    d.set_ams_filament(AmsFilamentSetting {
        ams_id,
        tray_id,
        slot_id: tray_id,
        tray_info_idx,
        tray_type: frag.base_type.clone(),
        // Stamp the fragment name into the sub-brand. Generic variants
        // (Silk / Matte / CF) all share one sentinel SKU + tray_type, so
        // this is the only field that carries the variant — the printer
        // stores and re-reports it (the RFID spools do the same), making
        // the sync round-trip lossless. See `resolve_bambu_identity`.
        tray_sub_brands: frag.display_name.clone(),
        tray_color,
        nozzle_temp_min: nozzle_temp_min as i32,
        nozzle_temp_max: nozzle_temp_max as i32,
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end error path for the test-connection command: an
    /// unreachable U1 host makes the connect-time
    /// `/machine/system_info` probe fail, so the command surfaces a
    /// non-empty reason instead of hanging or panicking. (No driver is
    /// registered — this exercises the transient build + teardown.)
    #[tokio::test]
    async fn test_connection_reports_failure_for_unreachable_u1() {
        // Port 1 has nothing listening → the HTTP probe is refused
        // fast, so connect() returns Err and the command reports it.
        let config = DriverConfig::U1 {
            host: "127.0.0.1".into(),
            port: 1,
        };
        let err = driver_test_connection(config, "unpaired-instance".into())
            .await
            .unwrap_err();
        assert!(!err.is_empty(), "expected a non-empty failure reason");
    }

    #[test]
    fn ams_address_derivation_reverses_resolve_bambu_indexing() {
        // Slot index -> (ams_id, tray_id), the inverse of
        // resolve_bambu's `unit_pos * 4 + tray.id`. Single-AMS A1 mini:
        // slots 0..3 are unit 0's trays.
        for slot_idx in 0usize..4 {
            assert_eq!(slot_idx / 4, 0, "slot {slot_idx} is on AMS unit 0");
            assert_eq!(slot_idx % 4, slot_idx, "tray id matches slot for unit 0");
        }
        // A second stacked unit would be slots 4..7 -> ams_id 1.
        assert_eq!(5 / 4, 1);
        assert_eq!(5 % 4, 1);
    }
}
