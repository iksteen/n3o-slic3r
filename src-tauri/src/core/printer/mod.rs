//! Printer drivers.
//!
//! Each driver owns its send path end-to-end, including any wrapping,
//! transformation, or metadata injection the target printer requires.
//! The slicer produces canonical G-code via libslic3r; the driver
//! decides what to do with it before transmission. Adding a future
//! printer is a self-contained exercise — write the driver, declare
//! its capabilities, no shared-code changes (PRD §8.2 note).

pub mod bambu;
pub mod instance;
pub mod instance_library;
pub mod instance_registry;
pub mod instance_storage;
pub mod profile;
pub mod registry;
pub mod snapmaker;

pub use instance::{
    BedRef, ConnectionInfo, ExtruderState, FeedKind, NozzleMaterial, NozzleSku,
    PrinterInstance, SlotBinding, SlotRef,
};
pub use instance_library::{
    bundled_instances, instance_id_for_vendor_profile, BAMBI_ID, SNAPPY_ID,
};
pub use instance_registry::{
    list_instances, lookup_instance, mutate_instance, set_slot_color, set_slot_filament,
    InstanceMutError,
};
pub use profile::{BoundingBox, PrinterProfile, Toolhead};
pub use registry::{bundled_catalog, default_binding, lookup, CatalogEntry};

use crate::core::filament::{bundled_catalog as filament_bundled_catalog, FilamentProfile};
use crate::core::profile_library::{list_filament_fragments, FilamentFragmentSummary};

/// Tauri command: list every printer instance the user has access to
/// (the bundled fixtures plus any future user-library entries). Drives
/// the printer picker.
#[tauri::command]
pub fn printer_instance_list() -> Vec<PrinterInstance> {
    list_instances()
}

/// Tauri command: snapshot a single instance by id. The slot-binding
/// panel reads the chosen plate's instance through this.
#[tauri::command]
pub fn printer_instance_get(id: String) -> Option<PrinterInstance> {
    lookup_instance(&id)
}

/// Tauri command: bind (or clear) the filament loaded in one
/// instance's slot. Emits `printer:instance_changed` so consumers
/// re-fetch.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_slot_filament(
    id: String,
    extruder_idx: usize,
    slot_idx: usize,
    filament_identity: Option<String>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_slot_filament(&id, extruder_idx, slot_idx, filament_identity)
        .map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &updated.id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(updated)
}

/// Tauri command: set (or clear) the user-assigned spool color on one
/// instance's slot. Hex string like `"#ff8800"`. Emits
/// `printer:instance_changed` so consumers refetch.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_slot_color(
    id: String,
    extruder_idx: usize,
    slot_idx: usize,
    color: Option<String>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated =
        set_slot_color(&id, extruder_idx, slot_idx, color).map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &updated.id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(updated)
}

/// Tauri command: the bundled filament catalog. Returns the
/// cascade-context profiles (currently just `Generic PLA` —
/// `base_type`-driven cascade-resolve fallback). Used by the
/// slice-input builder; the slot picker uses
/// [`filament_profile_list`] for the richer vendor-fragment
/// surface instead.
#[tauri::command]
pub fn filament_catalog_list() -> Vec<FilamentProfile> {
    filament_bundled_catalog()
}

/// Tauri command: bundled vendor filament fragments
/// (`profiles/vendor/<vendor>/filament/<slug>.toml`). Each entry
/// carries the slug (the wire `filament_identity` the slot panel
/// stores) plus a display label parsed out of the fragment's
/// `filament_settings_id` field, plus the base material type for
/// the picker swatch.
///
/// Drives the slot-binding panel's filament dropdown — what the
/// user actually picks from when binding a slot. The cascade-
/// context `filament_catalog_list` above is for the slice-time
/// resolver, not the picker.
#[tauri::command]
pub fn filament_profile_list() -> Vec<FilamentFragmentSummary> {
    list_filament_fragments()
}
