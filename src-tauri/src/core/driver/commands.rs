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

use std::sync::Arc;

use tauri::State;

use super::dryrun::neuter_gcode_3mf;
use super::registry::{DriverRegistry, DriverSummary};
use super::status::PrinterStatus;
use super::traits::{
    DriverConfig, DriverError, DriverId, SendHandle, SendPayload, PrinterCommand,
};

/// Register a fresh driver instance with the runtime. Returns
/// the allocated [`DriverId`] on success. Doesn't auto-connect
/// — caller follows up with [`driver_connect`].
///
/// Returns `DriverError::Other` until PR-7a-2 (Bambu) / PR-7b-2
/// (U1) land their concrete `Driver` impls.
#[tauri::command]
#[tracing::instrument(skip(registry))]
pub async fn driver_register(
    config: DriverConfig,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<DriverId, String> {
    let _ = (config, registry);
    Err(DriverError::Other(
        "no driver implementations yet — \
         register is wired but PR-7a-2 / PR-7b-2 are open"
            .into(),
    )
    .to_string())
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
