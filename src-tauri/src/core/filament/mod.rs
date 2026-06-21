//! Filament profile library and printer-state-to-profile resolution.
//!
//! Reads live filament state from each connected printer (AMS lite
//! via MQTT on Bambu; per-toolhead loaded filament via HTTP on U1),
//! resolves model material → physical slot bindings per (plate,
//! printer), detects mismatches (material family, temperature range,
//! color), and emits the right sync-on-send metadata for each printer
//! driver.
//!
//! Owns FR-FS-1 through FR-FS-14 (PRD §6.8). Live sync + driver wiring
//! land in Phase 7. Phase 1 ships only the declarative
//! `FilamentProfile` descriptor the cascade resolver reads via
//! `Context::predicate_value`.

pub mod library;
pub mod profile;
pub mod registry;

pub use library::UserFilament;
pub use profile::FilamentProfile;
pub use registry::{bundled_catalog, lookup};

use std::collections::HashMap;
use tauri::Emitter;

/// Emitted after any user-filament mutation so the frontend catalog +
/// open editor refetch (the `filament_catalog` query invalidates on it).
const FILAMENT_CHANGED: &str = "filament:changed";

fn emit_changed(window: &tauri::Window) {
    if let Err(e) = window.emit(FILAMENT_CHANGED, ()) {
        tracing::warn!(error = %e, "filament:changed emit failed");
    }
}

/// Fetch a bundled filament's user override profile (if it's been edited).
/// `None` when pristine — the editor then shows the bundled defaults with
/// no overrides.
#[tauri::command]
pub fn user_filament_get(base: String) -> Option<UserFilament> {
    library::lookup(&base)
}

/// Discard a filament's user overrides — back to pristine bundled. Drives
/// the picker's Revert affordance.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn user_filament_revert(base: String, window: tauri::Window) -> Result<(), String> {
    library::revert(&base);
    emit_changed(&window);
    Ok(())
}

/// Set (or clear, with `value = None`) one filament-bucket override on a
/// bundled filament, editing it in place. The override profile is created
/// on the first edit and removed once its last override is cleared. Rejects
/// non-Filament-bucket keys so process/printer keys can't be smuggled into
/// the filament tier.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn user_filament_set_override(
    base: String,
    key: String,
    value: Option<String>,
    window: tauri::Window,
) -> Result<UserFilament, String> {
    if slic3r_ffi::bucket_of(&key) != Some(slic3r_ffi::OptBucket::Filament) {
        return Err(format!("`{key}` is not a filament setting"));
    }
    let f = library::set_override(&base, key, value).map_err(|e| e.to_string())?;
    emit_changed(&window);
    Ok(f)
}

/// A filament's *base* (pre-override) scalar values — the editor shows
/// these beneath any override, same role as the machine panel's resolved
/// config.
#[tauri::command]
pub fn user_filament_resolved_config(base: String) -> Result<HashMap<String, String>, String> {
    Ok(crate::core::profile_library::resolve_base_scalars(&base)
        .into_iter()
        .collect())
}
