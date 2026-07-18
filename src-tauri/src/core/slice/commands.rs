//! Tauri command surface for the slice orchestrator.
//!
//! Thin layer over [`super::orchestrator::start_slice_job_with_sink_and_plugins`]
//! + [`super::job::JobRegistry`]. Tauri-managed state is the job registry
//! itself; the cascade is composed per-job inside the orchestrator.
//!
//! ## `slice_active_plate`
//!
//! The state-driven slice command. Takes an optional plate id
//! (defaulting to the project's active plate), builds a
//! [`SliceJobInput`](super::job::SliceJobInput) from project state via
//! [`super::input::build_slice_input`] (geometry rides in-memory as
//! `Arc`-shared buffers — no temp `.3mf`), and spawns the orchestrator.
//! Replaces the path-based `slice_start_default_a1mini` flow.

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-local sequence for naming slice output dirs — see
/// `slice_active_plate`. Avoids consuming a `JobId` just for a temp path.
static OUTPUT_DIR_SEQ: AtomicU64 = AtomicU64::new(0);
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use super::events::SliceEvent;
use super::input::{build_slice_input, SliceInputError};
use super::job::{JobId, JobRegistry, JobStatus};
use super::orchestrator::{
    start_slice_job_with_sink_and_plugins, EventSink, SliceStartError,
};
use super::pre_slice_gate::validate_pre_slice;
use crate::core::plugin::commands::PluginHostState;
use crate::core::project::{PlateId, Session};

/// Flip the cancel flag on a running job AND abort the in-flight libslic3r
/// `process()` (the long step) mid-flight. The flag covers between-plate
/// boundaries; `cancel_active_slice` aborts the plate currently slicing, so the
/// worker stops promptly instead of running the plate to completion. Either way
/// the worker emits `slice:cancelled` once it acknowledges.
#[tauri::command]
#[tracing::instrument(skip(jobs))]
pub fn slice_cancel(job_id: JobId, jobs: State<Arc<JobRegistry>>) -> Result<(), String> {
    let handle = jobs
        .get(job_id)
        .ok_or_else(|| format!("unknown job id {}", job_id.0))?;
    handle.cancel();
    // Abort the plate currently in process() (no-op if between plates / idle).
    slic3r_ffi::cancel_active_slice();
    Ok(())
}

/// Read the current cached status snapshot for a job. Lets the
/// frontend rebuild slice-panel state on reconnect without waiting
/// for the next progress tick.
#[tauri::command]
#[tracing::instrument(skip(jobs))]
pub fn slice_status(job_id: JobId, jobs: State<Arc<JobRegistry>>) -> Result<JobStatus, String> {
    let handle = jobs
        .get(job_id)
        .ok_or_else(|| format!("unknown job id {}", job_id.0))?;
    Ok(handle.snapshot())
}

/// Slice the active plate (or the requested `plate_id`) using live
/// project state. Builds a [`SliceJobInput`](super::job::SliceJobInput) from `project` via
/// [`build_slice_input`] (the plate's geometry rides in-memory as
/// `Arc`-shared buffers) and spawns the orchestrator.
///
/// Output `.gcode` lands in a per-job temp dir under
/// `std::env::temp_dir().join(format!("n3o-slice-{job_id}"))`.
/// The frontend reads the resulting path off `slice:plate_finished`
/// events when it wants to preview / send the result.
#[tauri::command]
#[tracing::instrument(skip(app_handle, jobs, session))]
pub async fn slice_active_plate(
    plate_id: Option<PlateId>,
    app_handle: AppHandle,
    jobs: State<'_, Arc<JobRegistry>>,
    session: State<'_, Arc<Mutex<Session>>>,
) -> Result<JobId, String> {
    // This command is `async` + the heavy prep runs on the blocking pool
    // ON PURPOSE: a *sync* Tauri command runs on the main (UI) thread, so the
    // build_slice_input prep (cascade compose + per-object assembly) plus the
    // project snapshot would contend with the UI for its duration. Off the
    // main thread, the window stays responsive while it runs. The Arc-shared
    // mesh buffers keep both the under-lock snapshot and the SliceObject
    // assembly cheap so scene mutations aren't blocked either.
    let session = Arc::clone(session.inner());
    let jobs = Arc::clone(jobs.inner());
    let input = tauri::async_runtime::spawn_blocking(move || {
        // Validate + resolve the plate + snapshot the project UNDER the lock,
        // then build the SliceJobInput OFF the lock (the snapshot is a cheap
        // Arc-bump of the geometry).
        let (snapshot, target_plate, output_dir) = {
            let s = session.lock().map_err(|e| format!("session lock: {e}"))?;
            let target_plate = plate_id.unwrap_or_else(|| {
                // Active plate; `Project::default()` invariant
                // guarantees `plates[active_plate]` is valid.
                s.project.plates[s.project.active_plate].id
            });
            // Pre-slice gate: refuse before any FS write if the
            // plate's material→slot map + bound PrinterInstance aren't
            // coherent. Returns the first failing plate's issue list as
            // a serialized SliceStartError::SliceBlocked.
            validate_pre_slice(&s.project, &[target_plate.0])
                .map_err(SliceStartError::SliceBlocked)
                .map_err(|e| e.to_string())?;
            // Opaque unique temp scope for this slice's G-code output. Named
            // from pid + a process-local sequence so it doesn't burn a real
            // `JobId` (the orchestrator allocs that) and stays unique across
            // concurrent slices and runs.
            let seq = OUTPUT_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
            let output_dir = std::env::temp_dir()
                .join(format!("n3o-slice-{}-{seq}", std::process::id()))
                .to_string_lossy()
                .into_owned();
            (s.project.clone(), target_plate, output_dir)
        };
        build_slice_input(&snapshot, target_plate, output_dir)
            .map_err(|e: SliceInputError| e.to_string())
    })
    .await
    .map_err(|e| format!("slice prep task panicked: {e}"))??;

    // Grab the plugin host (Arc clone) before app_handle moves into
    // the sink; `None` if it isn't managed (shouldn't happen in the
    // running app, but keeps this path test-friendly).
    let host = app_handle
        .try_state::<PluginHostState>()
        .map(|s| s.inner().clone());
    let sink = emit_sink(app_handle);
    start_slice_job_with_sink_and_plugins(input, &jobs, sink, host)
        .map_err(|e: SliceStartError| e.to_string())
}

/// Compose a sink that emits each lifecycle event on the AppHandle's
/// Tauri channel. Geometry is built in-memory, so the slice path leaves no
/// temp `.3mf` — nothing to clean up on the terminal event.
///
/// On `PlateFinished` the sink also stashes the sliced tower mesh straight into
/// the renderer (keyed by plate) *before* notifying the frontend, so the
/// frontend's re-render picks it up — the mesh never round-trips through TS.
fn emit_sink(app: AppHandle) -> EventSink {
    Box::new(move |event: SliceEvent| {
        if let SliceEvent::PlateFinished {
            plate_id,
            tower_mesh,
            ..
        } = &event
        {
            let pid = crate::core::project::PlateId(*plate_id);
            // The material count + printer the tower sliced at — the renderer
            // keeps the mesh only while these still match the resolved geometry.
            // Resolved under the project lock alone (released before the renderer
            // lock — no nested hold, so no lock-order coupling).
            let (material_count, printer) = {
                let session = app.state::<Arc<Mutex<crate::core::project::Session>>>();
                let s = session.lock().unwrap();
                crate::core::project::resolve::tower_geometry_for_plate(&s.project, pid)
                    .ok()
                    .flatten()
                    .map(|g| (g.material_count, g.printer_instance_id))
                    .unwrap_or((0, None))
            };
            let vp = app.state::<crate::viewport_render::ViewportState>();
            let mesh = tower_mesh
                .as_ref()
                .map(|m| (m.vertices.as_slice(), m.indices.as_slice()));
            crate::viewport_render::store_plate_tower_mesh(
                vp.inner(),
                pid,
                mesh,
                material_count,
                printer,
            );
        }
        let name = event.name();
        if let Err(e) = app.emit(name, &event) {
            tracing::warn!(event = name, error = %e, "slice event emit failed");
        }
    })
}
