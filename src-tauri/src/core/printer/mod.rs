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
    list_instances, lookup_instance, mutate_instance, set_slot_filament, InstanceMutError,
};
pub use profile::{BoundingBox, PrinterProfile, Toolhead};
pub use registry::{bundled_catalog, default_binding, lookup, CatalogEntry};

use crate::core::filament::{bundled_catalog as filament_bundled_catalog, FilamentProfile};

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

/// Tauri command: the bundled filament catalog. Drives the
/// per-slot filament picker. Currently a single Generic PLA entry
/// from the bundled fixtures — user-library filaments land in a
/// follow-up.
#[tauri::command]
pub fn filament_catalog_list() -> Vec<FilamentProfile> {
    filament_bundled_catalog()
}
