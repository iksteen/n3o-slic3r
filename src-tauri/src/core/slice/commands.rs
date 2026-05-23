//! Tauri command surface for the slice orchestrator (PR-3-2).
//!
//! Thin layer over [`super::orchestrator::start_slice_job`] +
//! [`super::job::JobRegistry`]. Tauri-managed state is the registry
//! itself; cascade lookups read the existing
//! `Mutex<CascadeRegistry>` shipped in PR-1-9.

use std::sync::Mutex;

use tauri::{AppHandle, State};

use super::job::{JobId, JobRegistry, JobStatus, SliceJobInput};
use super::orchestrator::{start_slice_job as run_start, SliceStartError};
use super::pre_slice_gate::validate_pre_slice;
use crate::core::cascade::CascadeRegistry;
use crate::core::project::Project;

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
    project: State<Mutex<Project>>,
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
