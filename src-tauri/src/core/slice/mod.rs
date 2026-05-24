//! Slice orchestration.
//!
//! FFI wrapper around `slic3r-ffi::slice`, slice progress reporting via
//! Tauri events, sequential multi-plate slice with overall progress,
//! per-plate output paths and time/filament summaries.
//!
//! Owns FR-SL-1 through FR-SL-5 (PRD §6.5). Module shape today:
//!
//! - **`summary`** (PR-3-3): `PlateSummary` + parser that builds it
//!   from libslic3r's emitted G-code header.
//! - **`errors`** (PR-3-3): `SliceError` + `classify_libslic3r_error`
//!   table-driven catalog with setting-key extraction.
//! - **`slicer_slice`** (Phase 0): one-shot synchronous command;
//!   does NOT use the cascade, the progress callback, or the
//!   summary/error pipeline. PR-3-2's orchestrator composes all of
//!   the above onto a worker thread; once that lands this command
//!   becomes the legacy debug-panel path and may be removed.

pub mod cascade_safety;
pub mod commands;
pub mod errors;
pub mod events;
pub mod input;
pub mod job;
pub mod orchestrator;
pub mod pre_slice_gate;
pub mod summary;

pub use commands::{slice_active_plate, slice_cancel, slice_start_job, slice_status};
pub use errors::{classify_libslic3r_error, SliceError};
pub use events::SliceEvent;
pub use job::{JobHandle, JobId, JobRegistry, JobStatus, SliceJobInput};
pub use orchestrator::{start_slice_job as start_slice_job_internal, SliceStartError};
pub use summary::{build_summary, build_summary_from_bytes, PlateSummary};

use serde::Serialize;
use slic3r_ffi::{slice, Config, Model};
use std::path::PathBuf;

#[derive(Serialize)]
pub struct SliceResult {
    pub ok: bool,
    pub out_path: String,
    pub error: Option<String>,
}

/// Load a model file and slice it with its embedded configuration (3MF)
/// or with FullPrintConfig defaults (STL/OBJ/STEP). Writes G-code to
/// `out_path`.
///
/// **Legacy path — Phase 0 surface.** Single synchronous call, blocks
/// the caller. Does NOT use the cascade resolver, does NOT fire the
/// progress callback (PR-3-1), does NOT classify errors or build a
/// `PlateSummary`. PR-3-2's orchestrator composes all of those onto
/// a worker thread; once that lands the debug panel migrates and
/// this command may be removed.
#[tauri::command]
#[tracing::instrument]
pub fn slicer_slice(model_path: String, out_path: String) -> SliceResult {
    let do_it = || -> Result<(), slic3r_ffi::Error> {
        let mut model = Model::new()?;
        let mut config = Config::new()?;
        model.load_with_config(PathBuf::from(&model_path), &mut config)?;
        slice(&model, &config, PathBuf::from(&out_path))?;
        Ok(())
    };
    match do_it() {
        Ok(()) => {
            tracing::info!(out = %out_path, "slice ok");
            SliceResult { ok: true, out_path, error: None }
        }
        Err(e) => {
            tracing::error!(error = %e, "slice failed");
            SliceResult { ok: false, out_path, error: Some(format!("{e}")) }
        }
    }
}
