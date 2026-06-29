//! User process (quality) profile overrides.
//!
//! Stamping the current quality settings onto a bundled process profile as
//! a per-user diff. The override library + persistence live in [`library`];
//! the stamp itself (which reads a plate's project-tier overrides) is a
//! project mutation and lives in `core::project::commands`. These commands
//! are the read + revert surface the Quality picker uses.

pub mod library;

pub use library::UserProcess;

use tauri::Emitter;

/// Emitted after any user-process mutation so the Quality picker refetches
/// its bold/Revert state and the panel re-resolves.
pub const PROCESS_CHANGED: &str = "process:user_changed";

pub(crate) fn emit_changed(window: &tauri::Window) {
    if let Err(e) = window.emit(PROCESS_CHANGED, ()) {
        tracing::warn!(error = %e, "process:user_changed emit failed");
    }
}

/// Fetch the user override profile for a bundled process on a printer, or
/// `None` when pristine. Drives the picker's bold name + Revert affordance.
#[tauri::command]
pub fn user_process_get(printer: String, base: String) -> Option<UserProcess> {
    library::lookup(&printer, &base)
}

// Revert (in-place) and Delete (named custom) both touch the bound plate —
// they optionally apply the profile's settings back to the plate's project
// tier before removing it, and Delete repoints the plate. They live with the
// other plate mutations in `core::project::commands`.
