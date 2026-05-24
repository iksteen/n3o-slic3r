//! Tauri command surface for the driver layer (PR-7a-1).
//!
//! Eight commands cover the registry + per-driver lifecycle.
//! `driver_register` is the only driver-kind-aware one — it
//! takes a [`DriverConfig`] variant and instantiates the right
//! `Driver` impl. Until PR-7a-2 / PR-7b-2 land concrete impls,
//! `driver_register` returns `DriverError::Other` — the trait +
//! registry are usable, just empty.
//!
//! Status updates emit on `driver:status_update` as a Tauri
//! event with payload `{ driver_id, status }`. Driver workers
//! (PR-7a-3 / PR-7b-3) hook the event emission into their
//! rate-limited `watch::Sender<PrinterStatus>` pipelines.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::bambu::connection::{BambuConfig, BambuDriver};
use super::dryrun::neuter_gcode_3mf;
use super::registry::{DriverRegistry, DriverSummary};
use super::status::PrinterStatus;
use super::traits::{
    Driver, DriverConfig, DriverError, DriverId, SendHandle, SendPayload, PrinterCommand,
};
use crate::core::project::{PlateId, Project};
use crate::core::slice::pre_slice_gate::ams_bindings_for_plate;
use crate::core::threemf::{fixture_input, write_sliced_3mf, AmsBinding};

/// Wire-shape for the `driver:status_update` Tauri event the
/// frontend's `useDriverStatus` hook subscribes to. Carries the
/// driver id so the hook can filter to just the panel's driver.
#[derive(Debug, Clone, Serialize)]
struct StatusUpdateEvent {
    driver_id: DriverId,
    status: PrinterStatus,
}

/// Spawn a tokio task that pumps a driver's internal
/// `watch::Receiver<PrinterStatus>` to a Tauri event. Lives for
/// the driver's lifetime — the watch channel closes when the
/// driver is dropped (driver_unregister + registry remove),
/// which ends the task naturally. Per-driver rate-limiting
/// happens in the driver's own worker (PR-7a-3); this bridge
/// just forwards every change without filtering.
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
                    StatusUpdateEvent {
                        driver_id,
                        status,
                    },
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
///
/// U1 path stubbed — PR-7b-2 lands it.
#[tauri::command]
#[tracing::instrument(skip(registry, app))]
pub async fn driver_register(
    config: DriverConfig,
    app: AppHandle,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<DriverId, String> {
    match config {
        DriverConfig::Bambu {
            host,
            access_code,
            serial,
        } => {
            let bambu_config = BambuConfig {
                host,
                access_code,
                serial,
            };
            // `register_with` allocates the id atomically with
            // insertion so the driver's internal `id()` matches
            // the registry's id (drivers use it for log spans +
            // outgoing protocol frames).
            let mut bridge_rx = None;
            let id = registry.register_with(|id| {
                let driver = BambuDriver::new(id, bambu_config);
                bridge_rx = Some(driver.subscribe_status());
                Box::new(driver) as Box<dyn Driver>
            });
            if let Some(rx) = bridge_rx {
                spawn_status_bridge(app, id, rx);
            }
            Ok(id)
        }
        DriverConfig::U1 { .. } => Err(DriverError::Other(
            "U1 driver not implemented yet — PR-7b-2 follow-up".into(),
        )
        .to_string()),
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
        let mut d = handle.lock().await;
        // Best-effort disconnect; we remove regardless of result.
        let _ = d.disconnect().await;
    }
    registry.remove(id);
    Ok(())
}

/// Cheap summary of every registered driver. Frontend uses this
/// to populate "which printers are configured?" panes without
/// triggering per-driver work.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub fn driver_list(registry: State<'_, Arc<DriverRegistry>>) -> Vec<DriverSummary> {
    registry.list()
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
    let mut d = handle.lock().await;
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
    let mut d = handle.lock().await;
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
    let d = handle.lock().await;
    Ok(d.status())
}

/// Upload + start a print on the driver. Returns the
/// [`SendHandle`] correlating the new job with subsequent
/// status events.
#[tauri::command]
#[tracing::instrument(skip(registry, payload), fields(payload_kind = ?std::mem::discriminant(&payload)))]
pub async fn driver_send(
    id: DriverId,
    payload: SendPayload,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<SendHandle, String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.lock().await;
    d.send(payload).await.map_err(|e| e.to_string())
}

/// Wrap a raw G-code file on disk into a Bambu-flavored
/// `.gcode.3mf` bundle byte buffer. Used by the plate-send
/// commands as a stub for PR-7c-7's full sync-on-send pipeline:
/// the bundle that PR-7c-7 emits will include per-AMS bindings
/// + project metadata; this one just gets the bytes packaged
/// well enough for the printer to read.
///
/// Runs on `spawn_blocking` because the writer is sync-IO + does
/// per-entry MD5 work; calling it from an async command without
/// the offload would stall the runtime.
async fn wrap_gcode_as_3mf(
    gcode_path: String,
    plate_id: u32,
    ams_bindings: Vec<AmsBinding>,
) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let gcode_bytes = std::fs::read(&gcode_path)
            .map_err(|e| format!("read gcode at {gcode_path}: {e}"))?;
        let mut input = fixture_input(plate_id, gcode_bytes);
        // Inject the per-plate AMS slot map (PR-S-7). For Bambi
        // standalone (1 slot, no AMS) this is `[{material: 1,
        // ams_slot: 1}]` — identity-shaped. For a future
        // AMS-equipped instance the picker drives the values.
        if let Some(plate) = input.plates.iter_mut().find(|p| p.plate_id == plate_id) {
            plate.ams_bindings = ams_bindings;
        }
        let tmp = tempfile::Builder::new()
            .suffix(".gcode.3mf")
            .tempfile()
            .map_err(|e| format!("create temp bundle: {e}"))?;
        write_sliced_3mf(&input, tmp.path())
            .map_err(|e| format!("write .gcode.3mf bundle: {e}"))?;
        std::fs::read(tmp.path())
            .map_err(|e| format!("read back .gcode.3mf bundle: {e}"))
    })
    .await
    .map_err(|e| format!("wrap task join: {e}"))?
}

/// Look up the active project's plate-side AMS bindings for use in
/// the send/dry-send path. Returns an empty vec when the plate isn't
/// found or has no mappings — both safe defaults the firmware
/// tolerates on a single-slot, no-AMS print.
fn collect_ams_bindings(
    project: &Mutex<Project>,
    plate_id: u32,
) -> Vec<AmsBinding> {
    let Ok(p) = project.lock() else {
        return Vec::new();
    };
    let Some(plate) = p.plate(PlateId(plate_id)) else {
        return Vec::new();
    };
    ams_bindings_for_plate(plate)
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
    project: State<'_, Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let ams = collect_ams_bindings(&project, plate_id);
    let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, ams).await?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&output_path, &bytes)
            .map_err(|e| format!("write {output_path}: {e}"))
    })
    .await
    .map_err(|e| format!("export task join: {e}"))?
}

/// Send the plate's last-sliced raw G-code to the driver as a
/// `.gcode.3mf` bundle. The frontend obtains `gcode_path` from
/// the most recent `slice:plate_finished` event (the `output_path`
/// field on `PlateSummary`).
///
/// Bundling is a stub: it uses [`fixture_input`] to produce a
/// minimal valid `.gcode.3mf` shell around the raw G-code. The
/// real sync-on-send pipeline (PR-7c-7) will embed per-AMS slot
/// bindings + project metadata; this command keeps the printer's
/// firmware happy in the meantime.
#[tauri::command]
#[tracing::instrument(skip(registry, project))]
pub async fn driver_send_plate(
    id: DriverId,
    plate_id: u32,
    gcode_path: String,
    registry: State<'_, Arc<DriverRegistry>>,
    project: State<'_, Arc<Mutex<Project>>>,
) -> Result<SendHandle, String> {
    let ams = collect_ams_bindings(&project, plate_id);
    let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, ams).await?;
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.lock().await;
    d.send(SendPayload::Gcode3mf { bytes, plate_id })
        .await
        .map_err(|e| e.to_string())
}

/// Dry-run variant of [`driver_send_plate`]. Same wrap pipeline,
/// then routes through [`neuter_gcode_3mf`] to strip extrusion +
/// comment out heater commands before send. Result: printer
/// exercises every XY motion cold, with zero filament flow.
#[tauri::command]
#[tracing::instrument(skip(registry, project))]
pub async fn driver_dry_send_plate(
    id: DriverId,
    plate_id: u32,
    gcode_path: String,
    registry: State<'_, Arc<DriverRegistry>>,
    project: State<'_, Arc<Mutex<Project>>>,
) -> Result<SendHandle, String> {
    let ams = collect_ams_bindings(&project, plate_id);
    let wrapped = wrap_gcode_as_3mf(gcode_path, plate_id, ams).await?;
    let neutered = tauri::async_runtime::spawn_blocking(move || neuter_gcode_3mf(&wrapped))
        .await
        .map_err(|e| format!("neuter task join: {e}"))?
        .map_err(|e| format!("neuter bundle: {e}"))?;
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.lock().await;
    d.send(SendPayload::Gcode3mf {
        bytes: neutered,
        plate_id,
    })
    .await
    .map_err(|e| e.to_string())
}

/// Motion-only dry-run variant of [`driver_send`]. Neuters the
/// payload's G-code (strips E values, comments out heater commands)
/// before forwarding to the driver — the printer goes through every
/// XY motion without heating or extruding. Use as the first send
/// against a newly-paired printer to confirm the toolpath without
/// risking the bed.
///
/// Only [`SendPayload::Gcode3mf`] is supported for now (Bambu path);
/// the U1 raw-G-code variant will follow when PR-7b-4 lands.
#[tauri::command]
#[tracing::instrument(skip(registry, payload), fields(payload_kind = ?std::mem::discriminant(&payload)))]
pub async fn driver_dry_send(
    id: DriverId,
    payload: SendPayload,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<SendHandle, String> {
    let neutered_payload = match payload {
        SendPayload::Gcode3mf { bytes, plate_id } => {
            let neutered = neuter_gcode_3mf(&bytes).map_err(|e| e.to_string())?;
            SendPayload::Gcode3mf {
                bytes: neutered,
                plate_id,
            }
        }
        SendPayload::Gcode { .. } => {
            return Err(
                "dry-run send for U1 raw-G-code payloads not implemented yet (PR-7b-4 follow-up)"
                    .into(),
            );
        }
    };
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let mut d = handle.lock().await;
    d.send(neutered_payload).await.map_err(|e| e.to_string())
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
    let mut d = handle.lock().await;
    d.command(cmd).await.map_err(|e| e.to_string())
}
