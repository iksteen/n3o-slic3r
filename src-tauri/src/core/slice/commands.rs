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
//! [`super::input::build_slice_input`], spawns the orchestrator with
//! a sink that cleans up the temp `.3mf` on the job's terminal
//! event. Replaces the path-based `slice_start_default_a1mini` flow.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
use crate::core::project::{PlateId, Project};

/// Flip the cancel flag on a running job. The worker thread reads
/// it between plates (and, when the FFI's mid-process cancel hook
/// lands, between progress ticks) and emits a `slice:cancelled`
/// event once it acknowledges the request.
#[tauri::command]
#[tracing::instrument(skip(jobs))]
pub fn slice_cancel(job_id: JobId, jobs: State<Arc<JobRegistry>>) -> Result<(), String> {
    let handle = jobs
        .get(job_id)
        .ok_or_else(|| format!("unknown job id {}", job_id.0))?;
    handle.cancel();
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
/// [`build_slice_input`], spawns the orchestrator, and registers a
/// cleanup hook so the temp `.3mf` gets deleted on the job's
/// terminal event.
///
/// Output `.gcode` lands in a per-job temp dir under
/// `std::env::temp_dir().join(format!("n3o-slice-{job_id}"))`.
/// The frontend reads the resulting path off `slice:plate_finished`
/// events when it wants to preview / send the result.
#[tauri::command]
#[tracing::instrument(skip(app_handle, jobs, project))]
pub async fn slice_active_plate(
    plate_id: Option<PlateId>,
    app_handle: AppHandle,
    jobs: State<'_, Arc<JobRegistry>>,
    project: State<'_, Arc<Mutex<Project>>>,
) -> Result<JobId, String> {
    // This command is `async` + the heavy prep runs on the blocking pool
    // ON PURPOSE: a *sync* Tauri command runs on the main (UI) thread, so the
    // build_slice_input prep — serializing the plate geometry and writing the
    // temp .3mf to disk, seconds for a large model — would freeze the whole UI
    // for its duration. Off the main thread, the window stays responsive while
    // it runs. The Arc-shared mesh buffers also keep the under-lock snapshot
    // cheap so scene mutations aren't blocked either.
    let project = Arc::clone(project.inner());
    let jobs = Arc::clone(jobs.inner());
    let (input, temp_path) = tauri::async_runtime::spawn_blocking(move || {
        // Validate + resolve the plate + snapshot the project UNDER the lock,
        // then build the SliceJobInput OFF the lock (the snapshot is a cheap
        // Arc-bump of the geometry; the disk write must not hold the mutex).
        let (snapshot, target_plate, output_dir) = {
            let p = project.lock().map_err(|e| format!("project lock: {e}"))?;
            let target_plate = plate_id.unwrap_or_else(|| {
                // Active plate; `Project::default()` invariant
                // guarantees `plates[active_plate]` is valid.
                p.plates[p.active_plate].id
            });
            // Pre-slice gate: refuse before any FS write if the
            // plate's material→slot map + bound PrinterInstance aren't
            // coherent. Returns the first failing plate's issue list as
            // a serialized SliceStartError::SliceBlocked.
            validate_pre_slice(&p, &[target_plate.0])
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
            (p.clone(), target_plate, output_dir)
        };
        build_slice_input(&snapshot, target_plate, output_dir)
            .map_err(|e: SliceInputError| e.to_string())
    })
    .await
    .map_err(|e| format!("slice prep task panicked: {e}"))??;

    // Sink wraps the standard AppHandle emit with a one-shot temp-
    // file cleanup that fires on the first terminal event. The
    // cleanup itself is best-effort — a missing file is a debug
    // log, not an error (the OS may have GC'd temp dirs).
    // Grab the plugin host (Arc clone) before app_handle moves into
    // the sink; `None` if it isn't managed (shouldn't happen in the
    // running app, but keeps this path test-friendly).
    let host = app_handle
        .try_state::<PluginHostState>()
        .map(|s| s.inner().clone());
    let sink = cleanup_sink(app_handle, temp_path.clone());
    start_slice_job_with_sink_and_plugins(input, &jobs, sink, host).map_err(
        |e: SliceStartError| {
            // Spawn failed → no worker, no terminal event, so
            // cleanup never fires. Best-effort delete here too.
            let _ = std::fs::remove_file(&temp_path);
            e.to_string()
        },
    )
}

/// Compose a sink that (a) emits each event on the AppHandle's Tauri
/// channel and (b) deletes `temp_path` exactly once, on the first
/// terminal event seen ([`SliceEvent::JobFinished`],
/// [`SliceEvent::JobFailed`], or [`SliceEvent::Cancelled`]).
///
/// The cleanup is fire-and-forget: a missing-file error is logged
/// at debug level, anything else at warn. The orchestrator never
/// reads the result.
fn cleanup_sink(app: AppHandle, temp_path: PathBuf) -> EventSink {
    let cleanup_done = Arc::new(AtomicBool::new(false));
    Box::new(move |event: SliceEvent| {
        let name = event.name();
        if let Err(e) = app.emit(name, &event) {
            tracing::warn!(event = name, error = %e, "slice event emit failed");
        }
        let terminal = matches!(
            event,
            SliceEvent::JobFinished { .. }
                | SliceEvent::JobFailed { .. }
                | SliceEvent::Cancelled { .. }
        );
        if terminal && !cleanup_done.swap(true, Ordering::SeqCst) {
            match std::fs::remove_file(&temp_path) {
                Ok(()) => {
                    tracing::debug!(
                        path = %temp_path.display(),
                        "slice temp file cleaned up",
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::debug!(
                        path = %temp_path.display(),
                        "slice temp file already gone at cleanup",
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        path = %temp_path.display(),
                        error = %e,
                        "slice temp file cleanup failed",
                    );
                }
            }
        }
    })
}
