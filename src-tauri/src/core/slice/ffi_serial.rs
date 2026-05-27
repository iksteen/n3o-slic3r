//! Process-wide serialization wrapper around `slic3r_ffi::slice`.
//!
//! libslic3r isn't generally thread-safe at the `Print::process()`
//! level — concurrent slices on heavier workloads SIGSEGV (multi-
//! material fourcolor benchy + cube-halves + snappy 4-color racing
//! through process() at once, observed in CI May 2026). The
//! `slic3r_ffi` crate intentionally stays a faithful binding to the
//! C surface and doesn't impose a serialization policy of its own;
//! this module is the application-side enforcement.
//!
//! Every n3o caller of `slic3r_ffi::slice` — the orchestrator, the
//! legacy single-shot command, the example binaries — routes
//! through [`slice`] here. The FFI's own tests are the only callers
//! that hit the raw entry point, and only because they're testing
//! the unwrapped surface (per-call callback isolation, concurrent
//! invocation safety of the trampoline itself).
//!
//! ## When the lock comes out
//!
//! If a future spike verifies libslic3r tolerates concurrent
//! `Print::process()` calls on production workloads (today's
//! evidence is only "20mm cube × 2 threads works 5/5" — see
//! `crates/slic3r-ffi/tests/api.rs::two_concurrent_slices_*`),
//! callers can move back to the raw FFI. Until then, every code
//! path that runs a slice goes through here.

use std::path::Path;
use std::sync::Mutex;

use slic3r_ffi::{slice as ffi_slice, Config, Model, Result};

/// Process-wide serialization mutex. Held for the duration of each
/// [`slice`] call so libslic3r runs one slice at a time across the
/// whole process — independent of how many threads, jobs, or test
/// binaries are in flight.
///
/// Poison recovery: a previous slice that panicked leaves the lock
/// poisoned but the inner `()` is meaningless, so we recover and
/// keep slicing.
static SLICE_LOCK: Mutex<()> = Mutex::new(());

/// Slice serially — acquires the process-wide [`SLICE_LOCK`] and
/// then forwards to [`slic3r_ffi::slice`].
///
/// Same signature and semantics as the raw FFI call (per-call
/// progress closure; closure fires synchronously on the slicing
/// thread); the only difference is that calls from multiple threads
/// queue rather than racing each other into libslic3r.
pub fn slice<P, F>(
    model: &Model,
    config: &Config,
    out_gcode_path: P,
    progress: F,
) -> Result<()>
where
    P: AsRef<Path>,
    F: FnMut(i32, &str),
{
    let _guard = SLICE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ffi_slice(model, config, out_gcode_path, progress)
}
