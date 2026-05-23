//! Slice orchestration.
//!
//! FFI wrapper around `slic3r-ffi::slice`, slice progress reporting via
//! Tauri events, sequential multi-plate slice with overall progress,
//! per-plate output paths and time/filament summaries.
//!
//! Owns FR-SL-1 through FR-SL-5 (PRD §6.5). For Phase 0 this hosts the
//! one-shot `slicer_slice` command; richer orchestration (off-UI-thread
//! progress callbacks, per-plate sequencing) lands in Phase 3.

pub mod errors;
pub mod summary;

pub use errors::{classify_libslic3r_error, SliceError};
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
/// Phase 0 surface — single call, blocks the calling thread. Progress
/// reporting and off-UI-thread execution come in Phase 3.
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
