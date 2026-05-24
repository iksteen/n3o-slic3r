//! Tauri command surface for the slice orchestrator (PR-3-2).
//!
//! Thin layer over [`super::orchestrator::start_slice_job`] +
//! [`super::job::JobRegistry`]. Tauri-managed state is the registry
//! itself; cascade lookups read the existing
//! `Mutex<CascadeRegistry>` shipped in PR-1-9.
//!
//! ## `slice_active_plate` (PR-6-2)
//!
//! The state-driven slice command. Takes an optional plate id
//! (defaulting to the project's active plate), builds a
//! [`SliceJobInput`] from project state via
//! [`super::input::build_slice_input`], spawns the orchestrator with
//! a sink that cleans up the temp `.3mf` on the job's terminal
//! event. Replaces the path-based `slice_start_default_a1mini` flow.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use super::events::SliceEvent;
use super::input::{build_slice_input, SliceInputError};
use super::job::{JobId, JobRegistry, JobStatus, SliceJobInput};
use super::orchestrator::{
    start_slice_job as run_start, start_slice_job_with_sink, EventSink,
    SliceStartError,
};
use super::pre_slice_gate::validate_pre_slice;
use crate::core::cascade::CascadeRegistry;
use crate::core::project::{PlateId, Project};

/// Kick off a slice job. Returns the allocated [`JobId`]
/// synchronously; the worker thread drives lifecycle events
/// through the Tauri event channel.
///
/// Pre-flight (before spawning the worker):
///   1. Material-binding validation against the requested plates'
///      bindings (PR-5-6 follow-up): `slice_start_job` reads the
///      `Project` state under its mutex and refuses to launch on
///      any unresolved binding issue. The frontend surfaces
///      `InvalidMaterialBindings` on the binding panel.
///   2. Cascade-handle resolution + output-dir writability —
///      delegated to the orchestrator.
#[tauri::command]
#[tracing::instrument(skip(app_handle, jobs, cascades, project, input))]
pub fn slice_start_job(
    input: SliceJobInput,
    app_handle: AppHandle,
    jobs: State<JobRegistry>,
    cascades: State<Mutex<CascadeRegistry>>,
    project: State<Arc<Mutex<Project>>>,
) -> Result<JobId, String> {
    // Pre-slice gate: refuse the job up front if any requested
    // plate's bindings are invalid. Quick, no-cost-to-the-user
    // failure mode that beats spawning the worker and erroring
    // mid-slice. Slot count comes from the resolved printer
    // profile the frontend passed in via context; PrinterProfile
    // stores it as `usize` but binding slot indices are `u8`, so
    // we cap at u8::MAX (256-slot printers don't exist).
    let slot_count = u8::try_from(input.context.printer.slot_count).unwrap_or(u8::MAX);
    {
        let p = project
            .lock()
            .map_err(|e| format!("project lock: {e}"))?;
        validate_pre_slice(&p, &input.plate_ids, slot_count)
            .map_err(SliceStartError::InvalidMaterialBindings)
            .map_err(|e: SliceStartError| e.to_string())?;
    }
    let cascades = cascades
        .lock()
        .map_err(|e| format!("cascade registry lock: {e}"))?;
    run_start(input, app_handle, jobs.inner(), &cascades).map_err(|e: SliceStartError| e.to_string())
}

/// Flip the cancel flag on a running job. The worker thread reads
/// it between plates (and, when the FFI's mid-process cancel hook
/// lands, between progress ticks) and emits a `slice:cancelled`
/// event once it acknowledges the request.
#[tauri::command]
#[tracing::instrument(skip(jobs))]
pub fn slice_cancel(job_id: JobId, jobs: State<JobRegistry>) -> Result<(), String> {
    let handle = jobs
        .get(job_id)
        .ok_or_else(|| format!("unknown job id {}", job_id.0))?;
    handle.cancel();
    Ok(())
}

/// Read the current cached status. Used by the renderer's
/// reconnect path to rebuild slice-panel UI without waiting for
/// the next progress tick.
#[tauri::command]
#[tracing::instrument(skip(jobs))]
pub fn slice_status(job_id: JobId, jobs: State<JobRegistry>) -> Result<JobStatus, String> {
    let handle = jobs
        .get(job_id)
        .ok_or_else(|| format!("unknown job id {}", job_id.0))?;
    Ok(handle.snapshot())
}

/// Slice the active plate (or the requested `plate_id`) using live
/// project state. Builds a [`SliceJobInput`] from `project` via
/// [`build_slice_input`], spawns the orchestrator, and registers a
/// cleanup hook so the temp `.3mf` gets deleted on the job's
/// terminal event.
///
/// Output `.gcode` lands in a per-job temp dir under
/// `std::env::temp_dir().join(format!("n3o-slice-{job_id}"))`.
/// The frontend reads the resulting path off `slice:plate_finished`
/// events when it wants to preview / send the result.
#[tauri::command]
#[tracing::instrument(skip(app_handle, jobs, cascades, project))]
pub fn slice_active_plate(
    plate_id: Option<PlateId>,
    app_handle: AppHandle,
    jobs: State<JobRegistry>,
    cascades: State<Mutex<CascadeRegistry>>,
    project: State<Arc<Mutex<Project>>>,
) -> Result<JobId, String> {
    // Self-heal a stale `Project.cascade_handle` before building
    // the input — autosave restore (PR-5-10) carries a handle
    // serialized from a prior session; the registry restarts each
    // process, so that handle wouldn't exist. ensure_default_
    // cascade_loaded is a no-op when the handle is live + bound.
    crate::core::cascade::commands::ensure_default_cascade_loaded(
        &*cascades,
        &**project,
        /* force_reinstall = */ false,
    )?;

    // Build the SliceJobInput + temp-file path under the project
    // mutex. We drop the lock before spawning the orchestrator so
    // the worker thread doesn't contend with frontend updates.
    let (input, temp_path) = {
        let p = project.lock().map_err(|e| format!("project lock: {e}"))?;
        let target_plate = plate_id.unwrap_or_else(|| {
            // Active plate; `Project::default()` invariant
            // guarantees `plates[active_plate]` is valid.
            p.plates[p.active_plate].id
        });
        let job_id_preview = jobs.alloc_id();
        // Re-allocate properly inside start_slice_job; this is just
        // for the output_dir name. The actual JobId may differ if
        // another command lands between the alloc and the spawn —
        // that's harmless, the dir's just an opaque temp scope.
        let output_dir = std::env::temp_dir()
            .join(format!("n3o-slice-{}", job_id_preview.0))
            .to_string_lossy()
            .into_owned();
        build_slice_input(&p, target_plate, output_dir)
            .map_err(|e: SliceInputError| e.to_string())?
    };

    // Pre-slice gate (PR-5-6): refuse if material bindings are
    // unresolvable. Same shape as slice_start_job's gate.
    let slot_count =
        u8::try_from(input.context.printer.slot_count).unwrap_or(u8::MAX);
    {
        let p = project.lock().map_err(|e| format!("project lock: {e}"))?;
        validate_pre_slice(&p, &input.plate_ids, slot_count)
            .map_err(SliceStartError::InvalidMaterialBindings)
            .map_err(|e: SliceStartError| {
                // Clean up the temp file before bubbling the error;
                // the orchestrator never spawned a worker so its
                // sink-based cleanup hook won't fire.
                let _ = std::fs::remove_file(&temp_path);
                e.to_string()
            })?;
    }

    let cascades = cascades
        .lock()
        .map_err(|e| format!("cascade registry lock: {e}"))?;

    // Sink wraps the standard AppHandle emit with a one-shot temp-
    // file cleanup that fires on the first terminal event. The
    // cleanup itself is best-effort — a missing file is a debug
    // log, not an error (the OS may have GC'd temp dirs).
    let sink = cleanup_sink(app_handle, temp_path.clone());
    start_slice_job_with_sink(input, jobs.inner(), &cascades, sink)
        .map_err(|e: SliceStartError| {
            // Spawn failed → no worker, no terminal event, so
            // cleanup never fires. Best-effort delete here too.
            let _ = std::fs::remove_file(&temp_path);
            e.to_string()
        })
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
