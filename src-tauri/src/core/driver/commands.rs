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
use super::registry::{DriverRegistry, DriverSummary};
use super::snapmaker::{U1Config, U1Driver};
use super::status::PrinterStatus;
use super::traits::{
    Driver, DriverConfig, DriverId, DriverKind, PrinterCommand, SendHandle, SendPayload,
};
use crate::core::plugin::commands::PluginHostState;
use crate::core::plugin::{DispatchGate, HookKind, PayloadKind, PreSendHook, SendTarget};
use crate::core::project::{PlateId, Project};
use crate::core::slice::pre_slice_gate::{ams_bindings_for_plate, ams_mapping_for_plate};
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
///
/// U1 path stubbed — PR-7b-2 lands it.
/// Construct the concrete driver for a [`DriverConfig`] variant.
/// Shared by [`driver_register`] (which inserts it into the registry +
/// spawns the status bridge) and [`driver_test_connection`] (which
/// drives a throwaway instance and discards it), so the per-kind
/// construction lives in one place.
fn build_driver(id: DriverId, config: DriverConfig) -> Box<dyn Driver> {
    match config {
        DriverConfig::Bambu { host, access_code } => {
            Box::new(BambuDriver::new(id, BambuConfig { host, access_code }))
        }
        DriverConfig::U1 { host, port } => Box::new(U1Driver::new(id, U1Config { host, port })),
    }
}

#[tauri::command]
#[tracing::instrument(skip(registry, app))]
pub async fn driver_register(
    config: DriverConfig,
    app: AppHandle,
    registry: State<'_, Arc<DriverRegistry>>,
) -> Result<DriverId, String> {
    // `register_with` allocates the id atomically with insertion so the
    // driver's internal `id()` matches the registry's id (drivers use
    // it for log spans + outgoing protocol frames).
    let mut bridge_rx = None;
    let id = registry.register_with(|id| {
        let driver = build_driver(id, config);
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
pub async fn driver_test_connection(config: DriverConfig) -> Result<(), String> {
    use super::status::ConnectionState;

    // Generous cap covering Bambu's ~5-8s MQTT handshake; U1's HTTP
    // probe is faster.
    const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    // Transient driver. DriverId(0) is fine — it's never inserted into
    // the registry; the id only tags log spans / outgoing frames.
    let mut driver = build_driver(DriverId(0), config);

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
/// `.gcode.3mf` bundle byte buffer. Minimum-viable packaging — the
/// bytes are well-formed enough for the printer to accept, but
/// per-AMS bindings and project-metadata enrichment are pending
/// the sync-on-send work (Phase 7c).
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
        let gcode_bytes =
            std::fs::read(&gcode_path).map_err(|e| format!("read gcode at {gcode_path}: {e}"))?;
        let mut input = fixture_input(plate_id, gcode_bytes);
        // Inject the per-plate AMS slot map. For Bambi
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
        std::fs::read(tmp.path()).map_err(|e| format!("read back .gcode.3mf bundle: {e}"))
    })
    .await
    .map_err(|e| format!("wrap task join: {e}"))?
}

/// Read a raw G-code file off disk into memory for the U1 send
/// path. Sliced bundles can be tens of megabytes — the read is
/// offloaded to a blocking thread to keep the Tauri runtime
/// responsive.
async fn read_gcode_bytes(gcode_path: String) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::read(&gcode_path).map_err(|e| format!("read gcode at {gcode_path}: {e}"))
    })
    .await
    .map_err(|e| format!("read task join: {e}"))?
}

/// Look up the active project's plate-side AMS bindings for use in
/// the send/dry-send path. Returns an empty vec when the plate isn't
/// found or has no mappings — both safe defaults the firmware
/// tolerates on a single-slot, no-AMS print.
fn collect_ams_bindings(project: &Mutex<Project>, plate_id: u32) -> Vec<AmsBinding> {
    let Ok(p) = project.lock() else {
        return Vec::new();
    };
    let Some(plate) = p.plate(PlateId(plate_id)) else {
        return Vec::new();
    };
    ams_bindings_for_plate(plate)
}

/// Plate-side AMS routing for the Bambu MQTT `project_file` print
/// command: `(use_ams, ams_mapping, ams_mapping2)`. Arrays are
/// sized to the plate's materials list length; empty when the
/// plate is unknown, unbound, or carries no materials — the
/// firmware falls back to the external spool in that case.
fn collect_ams_mapping(
    project: &Mutex<Project>,
    plate_id: u32,
) -> (
    bool,
    Vec<i8>,
    Vec<crate::core::slice::pre_slice_gate::AmsMappingV2>,
) {
    let default = (false, Vec::new(), Vec::new());
    let Ok(p) = project.lock() else {
        return default;
    };
    let Some(plate) = p.plate(PlateId(plate_id)) else {
        return default;
    };
    ams_mapping_for_plate(plate)
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
    // MQTT mapping isn't surfaced in the exported bundle, but pull it
    // anyway so the .gcode.3mf side stays consistent with what the
    // send path would emit.
    let ams = collect_ams_bindings(&project, plate_id);
    let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, ams).await?;
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
///   Bundling is a stub: it uses [`fixture_input`] to produce a
///   minimal valid `.gcode.3mf` shell around the raw G-code. The
///   real sync-on-send pipeline (PR-7c-7) will embed per-AMS slot
///   bindings + project metadata; this command keeps the printer's
///   firmware happy in the meantime.
/// - **U1** — ship the raw G-code body as [`SendPayload::Gcode`].
///   Moonraker stores it under the supplied file name and starts
///   the print in the same multipart upload (see
///   `core/driver/snapmaker/http.rs`).
#[tauri::command]
#[tracing::instrument(skip(registry, project, plugin_host))]
pub async fn driver_send_plate(
    id: DriverId,
    plate_id: u32,
    gcode_path: String,
    registry: State<'_, Arc<DriverRegistry>>,
    project: State<'_, Arc<Mutex<Project>>>,
    plugin_host: State<'_, PluginHostState>,
) -> Result<SendHandle, String> {
    let handle = registry
        .get(id)
        .ok_or_else(|| format!("unknown driver id {}", id.0))?;
    let kind = handle.lock().await.kind();
    let payload = match kind {
        DriverKind::Bambu => {
            let ams = collect_ams_bindings(&project, plate_id);
            let (use_ams, ams_mapping, ams_mapping2) = collect_ams_mapping(&project, plate_id);
            let bytes = wrap_gcode_as_3mf(gcode_path, plate_id, ams).await?;
            SendPayload::Gcode3mf {
                bytes,
                plate_id,
                use_ams,
                ams_mapping,
                ams_mapping2,
            }
        }
        DriverKind::U1 => {
            let bytes = read_gcode_bytes(gcode_path).await?;
            SendPayload::Gcode {
                bytes,
                file_name: format!("plate-{plate_id}.gcode"),
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
    let mut d = handle.lock().await;
    d.send(payload).await.map_err(|e| e.to_string())
}

/// Run the pre-send hook over `payload`, swapping in any plugin-edited
/// bytes. No-op when no plugin declares the hook; a panic in plugin Lua
/// is caught and the original bytes are sent unchanged.
/// Resolve the printer model bound to `plate_id`, for pre-send
/// `printer_compatibility` enforcement. `None` when the plate isn't
/// bound or the instance/profile can't be resolved — the printer check
/// is then simply skipped (the gate treats `None` as "any").
fn plate_printer_model(project: &Mutex<Project>, plate_id: u32) -> Option<String> {
    let inst_id = {
        let p = project.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        p.plate(PlateId(plate_id))?.printer_instance_id.clone()?
    };
    let inst = crate::core::printer::lookup_instance(&inst_id)?;
    let profile = crate::core::printer::lookup(&inst.vendor_profile_ref)?;
    Some(profile.model.clone())
}

fn apply_pre_send(
    host: &PluginHostState,
    payload: SendPayload,
    plate_id: u32,
    kind: DriverKind,
    printer_model: Option<String>,
) -> SendPayload {
    let lock = || host.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // Per-plate plugin activation doesn't apply to a whole-job send, so
    // the gate carries only the printer model (for compatibility);
    // global-default activation arrives with the plugin-state file.
    let gate = DispatchGate {
        printer_model,
        activation: Default::default(),
    };
    if !lock().any_active_hook(HookKind::PreSend, &gate) {
        return payload;
    }

    let (payload_kind, bytes) = match &payload {
        SendPayload::Gcode { bytes, .. } => (PayloadKind::Gcode, bytes.clone()),
        // A `.gcode.3mf` bundle is an opaque zip; letting a text-editing
        // plugin (e.g. one written for U1 raw G-code) rewrite its bytes
        // would silently corrupt the archive. Skip pre-send for it for
        // now — editing the bundle is an advanced, opt-in concern.
        SendPayload::Gcode3mf { .. } => return payload,
    };
    let hook = PreSendHook {
        kind: payload_kind,
        target: SendTarget {
            driver_kind: match kind {
                DriverKind::Bambu => "bambu".to_string(),
                DriverKind::U1 => "u1".to_string(),
            },
            plate_id,
        },
    };
    let edited = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lock().dispatch_gated(&hook, bytes.clone(), &gate)
    })) {
        Ok(b) => b,
        Err(_) => {
            tracing::error!("pre-send plugin hook panicked; sending unmodified payload");
            bytes
        }
    };

    match payload {
        SendPayload::Gcode { file_name, .. } => SendPayload::Gcode {
            bytes: edited,
            file_name,
        },
        SendPayload::Gcode3mf {
            plate_id,
            use_ams,
            ams_mapping,
            ams_mapping2,
            ..
        } => SendPayload::Gcode3mf {
            bytes: edited,
            plate_id,
            use_ams,
            ams_mapping,
            ams_mapping2,
        },
    }
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
        let err = driver_test_connection(config).await.unwrap_err();
        assert!(!err.is_empty(), "expected a non-empty failure reason");
    }

    fn host_with_pre_send(lua: &str) -> PluginHostState {
        use crate::core::plugin::PluginHost;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("p");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            "name=\"p\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks=[\"pre_send\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("main.lua"), lua).unwrap();
        // `load` reads the entry Lua into the runtime, so the temp dir
        // can drop right after.
        Arc::new(Mutex::new(PluginHost::load(&[tmp.path().to_path_buf()])))
    }

    #[test]
    fn apply_pre_send_rewrites_gcode_and_preserves_fields() {
        let host = host_with_pre_send(
            r#"function on_pre_send(p, t) return p.bytes .. "\n; via " .. t.driver_kind end"#,
        );
        let payload = SendPayload::Gcode {
            bytes: b"G1 X0".to_vec(),
            file_name: "plate-7.gcode".into(),
        };
        match apply_pre_send(&host, payload, 7, DriverKind::U1, None) {
            SendPayload::Gcode { bytes, file_name } => {
                assert_eq!(bytes, b"G1 X0\n; via u1".to_vec());
                assert_eq!(file_name, "plate-7.gcode", "file_name preserved");
            }
            other => panic!("expected Gcode, got {other:?}"),
        }
    }

    #[test]
    fn apply_pre_send_skips_gcode_3mf_bundle() {
        // Even a clobbering plugin can't touch the opaque bundle.
        let host = host_with_pre_send(r#"function on_pre_send(p, t) return "CLOBBERED" end"#);
        let original = vec![0x50, 0x4b, 0x03, 0x04]; // "PK\x03\x04" zip header
        let payload = SendPayload::Gcode3mf {
            bytes: original.clone(),
            plate_id: 3,
            use_ams: true,
            ams_mapping: vec![],
            ams_mapping2: vec![],
        };
        match apply_pre_send(&host, payload, 3, DriverKind::Bambu, None) {
            SendPayload::Gcode3mf {
                bytes,
                plate_id,
                use_ams,
                ..
            } => {
                assert_eq!(bytes, original, ".gcode.3mf bytes must be untouched");
                assert_eq!(plate_id, 3);
                assert!(use_ams);
            }
            other => panic!("expected Gcode3mf, got {other:?}"),
        }
    }
}
