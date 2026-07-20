//! Slice orchestration.
//!
//! FFI wrapper around `slic3r-ffi::slice`, slice progress reporting via
//! Tauri events, sequential multi-plate slice with overall progress,
//! per-plate output paths and time/filament summaries.
//!
//! Owns FR-SL-1 through FR-SL-5 (PRD §6.5). Module shape today:
//!
//! - **`summary`**: `PlateSummary` + parser that builds it
//!   from libslic3r's emitted G-code header.
//! - **`errors`**: `SliceError` + `classify_libslic3r_error`
//!   table-driven catalog with setting-key extraction.

pub mod cascade_safety;
pub mod commands;
pub mod errors;
pub mod events;
pub mod input;
pub mod job;
pub mod orchestrator;
pub mod pa_calibration;
pub mod pre_slice_gate;
pub mod summary;

pub use commands::{slice_active_plate, slice_cancel, slice_status};
pub use errors::{classify_libslic3r_error, SliceError};
pub use events::SliceEvent;
pub use job::{JobHandle, JobId, JobRegistry, JobStatus, SliceJobInput};
pub use orchestrator::SliceStartError;
pub use summary::{build_summary, build_summary_from_bytes, PlateSummary};
