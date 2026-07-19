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
use crate::core::project::Session;

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
#[tracing::instrument(skip(session))]
pub async fn driver_export_plate(
    plate_id: u32,
    gcode_path: String,
    output_path: String,
    thumbnail_png_base64: Option<String>,
    session: State<'_, Arc<Mutex<Session>>>,
) -> Result<(), String> {
    // MQTT mapping isn't surfaced in the exported bundle, but pull it
    // anyway so the .gcode.3mf side stays consistent with what the
    // send path would emit.
    let ams = collect_ams_bindings(&session, plate_id);
    let (_basename, title) = derive_send_names(&session, plate_id);
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
#[tracing::instrument(skip(registry, session, plugin_host, app, sends))]
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
    session: State<'_, Arc<Mutex<Session>>>,
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
    let (basename, title) = derive_send_names(&session, plate_id);
    // The bound instance's sticky per-print toggles (the send dialog
    // edits them; the drivers translate to their wire fields).
    let options = plate_send_options(&session, plate_id);
    // Captured before `options` moves into the payload: whether the user asked
    // the printer to run its own flow calibration this print. When they didn't,
    // we push our stored per-color K instead (below, before send).
    let flow_cali = options.flow_calibration;
    let payload = match kind {
        DriverKind::Bambu => {
            let ams = collect_ams_bindings(&session, plate_id);
            let (use_ams, ams_mapping, ams_mapping2) = collect_ams_mapping(&session, plate_id);
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
                    let map_table = u1_map_table(&session, plate_id);
                    let physical_nozzles = plate_nozzle_diameters(&session, plate_id);
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
    let printer_model = plate_printer_model(&session, plate_id);
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

    // Pre-print: push our stored per-color K to the printer so it applies the
    // right value per tray — UNLESS the user asked the printer to run its own
    // flow calibration this print (which would ignore/overwrite a pushed K).
    // Best-effort: a push failure logs and the print proceeds with the
    // printer's own table, rather than blocking the job.
    if matches!(kind, DriverKind::Bambu) && !flow_cali {
        // filament_id per tray from the live AMS report (the loaded spool's id).
        let loaded = bambu_loaded_filament_ids(&d.status());
        let mut entries = collect_cali_trays(&session, plate_id, &loaded);
        if !entries.is_empty() {
            // Reuse our profile's cali_idx if the printer already has it (match
            // on OUR setting_id, present after the first push) so we update in
            // place instead of creating a duplicate; leave None to create.
            match d.get_extrusion_cali(entries[0].nozzle_diameter.clone()).await {
                Ok(table) => {
                    for e in &mut entries {
                        e.cali_idx = table
                            .iter()
                            .find(|p| p.setting_id == e.setting_id)
                            .map(|p| p.cali_idx);
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "extrusion_cali_get before send failed; creating fresh profiles")
                }
            }
            if let Err(err) = d.set_extrusion_cali(entries).await {
                tracing::warn!(error = %err, "pre-print extrusion_cali_set failed; printing with the printer's own K");
            }
        }
    }

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

/// Run a pressure-advance calibration for a slot's filament and store the
/// measured K keyed by `(filament identity, spool color, nozzle)` so future
/// slices use it over the profile default. Reads identity/color from the slot
/// and nozzle diameter from its toolhead. The driver selects the slot's
/// toolhead before calibrating. Long-running (heats + sweeps — minutes).
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_calibrate_pa(
    driver_id: DriverId,
    instance_id: String,
    extruder_idx: usize,
    slot_idx: usize,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<f64, String> {
    use crate::core::printer::instance_registry::{
        lookup_instance, set_calibrated_pressure_advance,
    };

    let inst =
        lookup_instance(&instance_id).ok_or_else(|| format!("unknown instance {instance_id}"))?;
    let extruder = inst
        .extruders
        .get(extruder_idx)
        .ok_or_else(|| format!("extruder {extruder_idx} out of range"))?;
    let slot = extruder
        .slots
        .get(slot_idx)
        .ok_or_else(|| format!("slot {extruder_idx}/{slot_idx} out of range"))?;
    let identity = slot
        .filament_identity
        .clone()
        .ok_or("slot has no filament bound")?;
    let color = slot.color.clone().unwrap_or_default();
    let nozzle = extruder.installed_nozzle.diameter.clone();

    let k = {
        let handle = registry
            .get(driver_id)
            .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
        let d = handle.read().await;
        d.calibrate_pressure_advance(extruder_idx)
            .await
            .map_err(|e| e.to_string())?
    };

    set_calibrated_pressure_advance(&instance_id, identity, color, nozzle, Some(k))
        .map_err(|e| e.to_string())?;
    Ok(k)
}

/// Park the active toolhead back in its dock. Called once after a calibration
/// cycle finishes so the machine isn't left holding the last picked toolhead.
/// No-op on printers that aren't toolchangers.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_park_extruder(
    driver_id: DriverId,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<(), String> {
    let handle = registry
        .get(driver_id)
        .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
    let d = handle.read().await;
    d.park_extruder().await.map_err(|e| e.to_string())
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

/// Whether this instance's topology is supported for Bambu PA. We support
/// single-extruder printers with any number of AMS units (slot ↔ physical AMS
/// address is `ams_id = slot/4`, `tray_id = slot%4`, per sync.rs). **Dual-nozzle
/// H2D is refused** — its `extruder_id`/per-nozzle handling isn't done. Multi-AMS
/// addressing is correct but unverified on hardware (only the A1 mini was
/// tested); the one soft spot is result-matching, which relies on the printer
/// echoing `ams_id` in `extrusion_cali_get_result` (see the match below).
fn bambu_pa_topology_ok(inst: &crate::core::printer::PrinterInstance) -> bool {
    inst.extruders.len() == 1
}

/// Bambu composite nozzle id, e.g. `"HS00-0.4"` — mirrors Studio's
/// `_generate_nozzle_id`: `"H"` + flow-type char (`HighFlow → "H"`, else
/// `Standard → "S"`) + `"00-"` + diameter.
fn bambu_nozzle_id(material: &crate::core::printer::NozzleMaterial, diameter: &str) -> String {
    use crate::core::printer::NozzleMaterial::{HighFlowHardened, HighFlowStainless};
    let flow = if matches!(material, HighFlowHardened | HighFlowStainless) {
        "H"
    } else {
        "S"
    };
    format!("H{flow}00-{diameter}")
}

/// One measured K from a Bambu batched calibration, mapped back to its slot.
#[derive(serde::Serialize)]
pub struct BambuCaliSlotK {
    pub extruder_index: usize,
    pub slot_index: usize,
    pub k_value: f64,
    pub confidence: i32,
}

/// Run Bambu Flow-Dynamics calibration for a set of slots in one job, storing
/// each measured K in the instance's color-keyed store (rounded to 0.001, like
/// Studio) and returning it per slot. Resolves the Bambu `setting_id` per
/// filament from the printer's own cali table (`extrusion_cali_get`).
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_calibrate_pa_bambu(
    driver_id: DriverId,
    instance_id: String,
    slots: Vec<(usize, usize)>,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<Vec<BambuCaliSlotK>, String> {
    use crate::core::driver::traits::ExtrusionCaliTarget;
    use crate::core::printer::instance_registry::{
        lookup_instance, set_calibrated_pressure_advance,
    };
    use crate::core::profile_library::{list_filament_fragments, resolve_base_scalars};

    let inst =
        lookup_instance(&instance_id).ok_or_else(|| format!("unknown instance {instance_id}"))?;
    // Single-extruder only (dual-nozzle H2D unsupported); multi-AMS is
    // addressed correctly but unverified on hardware.
    if !bambu_pa_topology_ok(&inst) {
        return Err("Bambu PA calibration doesn't support dual-nozzle printers yet".into());
    }
    let library = list_filament_fragments();

    struct Meta {
        ext: usize,
        slot: usize,
        identity: String,
        color: String,
        nozzle: String,
        filament_id: String,
        ams_id: u8,
        tray_id: u8,
        nozzle_id: String,
        nozzle_temp: i32,
        bed_temp: i32,
        max_vol: String,
    }
    let mut metas: Vec<Meta> = Vec::new();
    for (ext_idx, slot_idx) in &slots {
        let ext = inst
            .extruders
            .get(*ext_idx)
            .ok_or_else(|| format!("extruder {ext_idx} out of range"))?;
        let slot = ext
            .slots
            .get(*slot_idx)
            .ok_or_else(|| format!("slot {ext_idx}/{slot_idx} out of range"))?;
        let identity = slot
            .filament_identity
            .clone()
            .ok_or("slot has no filament bound")?;
        let frag = library
            .iter()
            .find(|f| f.identity == identity)
            .ok_or_else(|| format!("filament '{identity}' not in library"))?;
        let filament_id = frag
            .filament_id
            .clone()
            .ok_or_else(|| format!("filament '{identity}' has no Bambu SKU"))?;
        let nozzle = ext.installed_nozzle.diameter.clone();
        let max_vol = resolve_base_scalars(&identity)
            .get("filament_max_volumetric_speed")
            .cloned()
            .unwrap_or_else(|| "12".to_owned());
        metas.push(Meta {
            ext: *ext_idx,
            slot: *slot_idx,
            identity,
            color: slot.color.clone().unwrap_or_default(),
            nozzle: nozzle.clone(),
            filament_id,
            ams_id: (*slot_idx / 4) as u8,
            tray_id: (*slot_idx % 4) as u8,
            nozzle_id: bambu_nozzle_id(&ext.installed_nozzle.material, &nozzle),
            nozzle_temp: frag.nozzle_temp as i32,
            bed_temp: frag.bed_temp as i32,
            max_vol,
        });
    }
    if metas.is_empty() {
        return Ok(Vec::new());
    }
    let nozzle_diameter = metas[0].nozzle.clone();

    let handle = registry
        .get(driver_id)
        .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
    // Known limitation: the read guard is held across the multi-minute
    // calibration job (like the U1 path). A mid-job disconnect would queue
    // behind it; a lock-free long-op design is a broader follow-up.
    let d = handle.read().await;

    // Resolve setting_id per filament_id from the printer's stored table,
    // preferring a current entry over a history one. Empty when the printer has
    // never calibrated this filament — it then creates a fresh profile.
    let table = d
        .get_extrusion_cali(nozzle_diameter)
        .await
        .map_err(|e| e.to_string())?;
    let setting_id_for = |fid: &str| -> String {
        table
            .iter()
            .filter(|p| p.filament_id == fid)
            .min_by_key(|p| u8::from(p.is_history))
            .map(|p| p.setting_id.clone())
            .unwrap_or_default()
    };

    let targets = metas
        .iter()
        .map(|m| ExtrusionCaliTarget {
            ams_id: m.ams_id,
            tray_id: m.tray_id,
            slot_id: m.tray_id,
            extruder_id: 0,
            filament_id: m.filament_id.clone(),
            setting_id: setting_id_for(&m.filament_id),
            nozzle_id: m.nozzle_id.clone(),
            nozzle_diameter: m.nozzle.clone(),
            nozzle_temp: m.nozzle_temp,
            bed_temp: m.bed_temp,
            max_volumetric_speed: m.max_vol.clone(),
        })
        .collect();

    let results = d
        .calibrate_pressure_advance_bambu(targets)
        .await
        .map_err(|e| e.to_string())?;
    drop(d);

    // Match on (ams_id, tray_id). On single-AMS both sides are ams_id 0. On
    // multi-AMS this relies on the printer echoing ams_id in the result; if it
    // doesn't (defaults 0), only unit-0 trays match and the rest surface as "no
    // result" rather than getting a wrong K — a safe degradation (untested).
    let mut out = Vec::new();
    for m in &metas {
        let Some(r) = results
            .iter()
            .find(|r| r.ams_id as u8 == m.ams_id && r.tray_id as u8 == m.tray_id)
        else {
            continue;
        };
        let k = (r.k_value * 1000.0).round() / 1000.0;
        // Only persist a SUCCESSFUL measurement (confidence 0 = ok, 1 =
        // uncertain, 2 = failed). A failed/low-confidence K is returned to the
        // UI (which marks the row failed) but never stored or pushed.
        if r.confidence == 0 {
            set_calibrated_pressure_advance(
                &instance_id,
                m.identity.clone(),
                m.color.clone(),
                m.nozzle.clone(),
                Some(k),
            )
            .map_err(|e| e.to_string())?;
        }
        out.push(BambuCaliSlotK {
            extruder_index: m.ext,
            slot_index: m.slot,
            k_value: k,
            confidence: r.confidence,
        });
    }
    Ok(out)
}

/// Stable, self-owned Bambu preset id for one of our K profiles. Deterministic
/// per `(identity, color, nozzle)` so we can find our own profile in the
/// printer's table again (to reuse its `cali_idx` and avoid duplicates). `"PF"`
/// mirrors Bambu's user-preset ids; the `"N3O"` tag avoids colliding with them.
fn bambu_setting_id(identity: &str, color: &str, nozzle: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a
    for b in format!("{identity}|{color}|{nozzle}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("PFN3O{h:016x}")
}

/// `(ams_id, tray_id) -> physically-loaded filament id` from the driver's live
/// AMS report (`tray_info_idx`). The pushed profile should carry the spool's
/// real id, not n3o's fragment SKU (which mismatches user filaments). Keyed by
/// unit POSITION (not `unit.id`) to match sync's `slot = unit_pos*4 + tray.id`
/// convention, so `(slot/4, slot%4)` looks up the right tray.
fn bambu_loaded_filament_ids(status: &PrinterStatus) -> HashMap<(u8, u8), String> {
    use crate::core::driver::status::DriverExtra;
    let mut map = HashMap::new();
    if let DriverExtra::Bambu(extra) = &status.extra {
        if let Some(ams) = &extra.ams {
            for (unit_pos, unit) in ams.units.iter().enumerate() {
                for tray in &unit.trays {
                    if let Some(fid) = tray.identity.as_ref().and_then(|f| f.filament_id.clone()) {
                        map.insert((unit_pos as u8, tray.id), fid);
                    }
                }
            }
        }
    }
    map
}

/// Collect the per-tray K entries to push before a Bambu print: one
/// [`ExtrusionCaliEntry`] per AMS-fed slot on the plate that has a stored
/// calibrated K, targeting the physical `(ams_id, tray_id) = (slot/4, slot%4)`
/// (sync's convention — correct for multi-AMS). `setting_id` is ours (stable);
/// `filament_id` is the physically-loaded `tray_info_idx` (from `loaded`),
/// falling back to n3o's fragment SKU. `cali_idx` is left `None`; the caller
/// resolves it from the table by our `setting_id`.
fn collect_cali_trays(
    session: &Mutex<Session>,
    plate_id: u32,
    loaded: &HashMap<(u8, u8), String>,
) -> Vec<crate::core::driver::traits::ExtrusionCaliEntry> {
    use crate::core::driver::traits::ExtrusionCaliEntry;
    use crate::core::printer::instance_registry::lookup_instance;
    use crate::core::printer::FeedKind;
    use crate::core::profile_library::list_filament_fragments;
    use crate::core::project::model::PlateId;

    let Ok(s) = session.lock() else {
        return Vec::new();
    };
    let Some(plate) = s.project.plate(PlateId(plate_id)) else {
        return Vec::new();
    };
    let Some(instance) = plate.printer_instance_id().and_then(lookup_instance) else {
        return Vec::new();
    };
    // Single-extruder only (dual-nozzle H2D unsupported); others get no push.
    if !bambu_pa_topology_ok(&instance) {
        return Vec::new();
    }
    let library = list_filament_fragments();

    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for slot_ref in plate.material_to_slot.values() {
        let Some(ext) = instance.extruders.get(slot_ref.extruder as usize) else {
            continue;
        };
        let Some(slot) = ext.slots.get(slot_ref.slot as usize) else {
            continue;
        };
        if slot.feed != FeedKind::Ams {
            continue;
        }
        let Some(identity) = slot.filament_identity.clone() else {
            continue;
        };
        let color = slot.color.clone().unwrap_or_default();
        let nozzle = ext.installed_nozzle.diameter.clone();
        let Some(k) = instance
            .calibrated_pressure_advance
            .get(&identity)
            .and_then(|c| c.get(&color))
            .and_then(|n| n.get(&nozzle))
            .copied()
        else {
            continue;
        };
        // Physical AMS address (sync's convention: slot = unit_pos*4 + tray.id).
        let ams_id = (slot_ref.slot / 4) as u8;
        let tray_id = (slot_ref.slot % 4) as u8;
        if !seen.insert((ams_id, tray_id)) {
            continue;
        }
        let frag = library.iter().find(|f| f.identity == identity);
        // Loaded spool's id (what the printer keys its table on), else the
        // fragment SKU. Skip if we have neither.
        let Some(filament_id) = loaded
            .get(&(ams_id, tray_id))
            .cloned()
            .or_else(|| frag.and_then(|f| f.filament_id.clone()))
        else {
            continue;
        };
        entries.push(ExtrusionCaliEntry {
            ams_id,
            tray_id,
            slot_id: tray_id,
            extruder_id: 0,
            filament_id,
            setting_id: bambu_setting_id(&identity, &color, &nozzle),
            name: frag.map(|f| f.display_name.clone()).unwrap_or_default(),
            nozzle_id: bambu_nozzle_id(&ext.installed_nozzle.material, &nozzle),
            nozzle_diameter: nozzle,
            k_value: k,
            n_coef: 1.0,
            cali_idx: None,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bambu_setting_id_is_stable_and_distinct() {
        // Deterministic (same inputs → same id, across calls) so we can find
        // our own profile in the printer's table again; distinct per profile
        // key; carries the collision-avoiding prefix.
        let a = bambu_setting_id("bambu-pla-silk", "#ec984c", "0.4");
        assert_eq!(a, bambu_setting_id("bambu-pla-silk", "#ec984c", "0.4"));
        assert!(a.starts_with("PFN3O"), "id={a}");
        assert_ne!(a, bambu_setting_id("bambu-pla-silk", "#000000", "0.4"));
        assert_ne!(a, bambu_setting_id("bambu-pla-silk", "#ec984c", "0.6"));
        assert_ne!(a, bambu_setting_id("generic-pla", "#ec984c", "0.4"));
    }

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
