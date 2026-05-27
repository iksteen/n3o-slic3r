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
pub mod sync;

pub use instance::{
    BedRef, ConnectionInfo, ExtruderState, FeedKind, NozzleMaterial, NozzleSku,
    PrinterInstance, SlotBinding, SlotRef,
};
pub use instance_library::{
    bundled_instances, instance_id_for_vendor_profile, BAMBI_ID, SNAPPY_ID,
};
pub use instance_registry::{
    create_instance, delete_instance, list_instances, lookup_instance, mutate_instance,
    set_extruder_nozzle_diameter, set_instance_bed, set_slot_color, set_slot_filament,
    InstanceMutError,
};
pub use profile::{BoundingBox, PrinterProfile, Toolhead};
pub use registry::{bundled_catalog, default_printer_identity, lookup, CatalogEntry};

use crate::core::filament::{bundled_catalog as filament_bundled_catalog, FilamentProfile};
use crate::core::profile_library::{list_filament_fragments, FilamentFragmentSummary};

/// Tauri command: list every printer instance the user has access
/// to — i.e. whatever the user library on disk currently holds.
/// Empty on first launch (the frontend renders the onboarding
/// empty-state in that case). Drives the printer picker.
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

/// Tauri command: register a new `PrinterInstance` from a bundled
/// printer identity + display name + AMS unit count. Returns the
/// new instance for the frontend to optionally auto-bind. Emits
/// `printer:instance_changed` so consumers re-list.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_create(
    printer_identity: String,
    display_name: String,
    ams_units: u32,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let inst = create_instance(&printer_identity, display_name, ams_units)
        .map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &inst.id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(inst)
}

/// Tauri command: remove a `PrinterInstance`. The caller (frontend)
/// is responsible for unbinding any plates that referenced this
/// instance; the backend currently leaves them as dangling refs
/// (the slice gate refuses to run, the picker surfaces "unbound").
/// Emits `printer:instance_changed` so consumers re-list.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_delete(
    id: String,
    window: tauri::Window,
) -> Result<(), String> {
    delete_instance(&id).map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(())
}

/// Tauri command: change the diameter of the nozzle currently
/// installed on the named extruder. Material is preserved (the picker
/// only surfaces diameter swaps in the MVP). Emits
/// `printer:instance_changed` so the cascade preview re-resolves.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_extruder_nozzle_diameter(
    id: String,
    extruder_idx: usize,
    diameter_mm: f32,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_extruder_nozzle_diameter(&id, extruder_idx, diameter_mm)
        .map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &updated.id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(updated)
}

/// Tauri command: change the bed currently loaded on one instance.
/// Validates against the bound printer profile's
/// `supported_build_plates`. Emits `printer:instance_changed` so the
/// slicer-composer + cascade preview re-resolve. This is the
/// single source of truth for "which bed is on this printer" — the
/// per-plate binding no longer carries a bed.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_bed(
    id: String,
    bed_identity: String,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated =
        set_instance_bed(&id, bed_identity).map_err(|e| e.to_string())?;
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

/// Tauri command: the bundled filament catalog — the cascade-
/// context profiles (currently just `Generic PLA`, the
/// `base_type`-driven cascade-resolve fallback). The slot picker
/// reads [`filament_profile_list`] instead for the richer vendor-
/// fragment surface.
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

/// Tauri command: pull the named printer's current spool loadout
/// from the live driver into the instance's SlotBindings (PR-7c-2).
///
/// Resolver lives in [`crate::core::printer::sync`]; this command
/// is the I/O wrapper — grab a status snapshot, project it into
/// per-slot updates, write through `mutate_instance` so it's one
/// atomic transaction that emits a single
/// `printer:instance_changed` event.
///
/// Errors: unknown driver id, unknown instance id, driver returns
/// no extra (shouldn't happen — every kind populates one). A
/// driver that's disconnected still returns a cached status; the
/// resolver simply finds no `ams` / empty `toolhead_filaments` and
/// produces no updates. The frontend treats "no driver registered"
/// as the disconnected case and renders the error triangle on the
/// sync button without invoking this command.
#[tauri::command]
#[tracing::instrument(skip(registry, window))]
pub async fn printer_instance_sync_from_driver(
    instance_id: String,
    driver_id: crate::core::driver::traits::DriverId,
    registry: tauri::State<
        '_,
        std::sync::Arc<crate::core::driver::registry::DriverRegistry>,
    >,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let handle = registry
        .get(driver_id)
        .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
    let status = { handle.lock().await.status() };
    let library = list_filament_fragments();
    let instance = lookup_instance(&instance_id)
        .ok_or_else(|| format!("unknown instance id {}", instance_id))?;
    let updates =
        crate::core::printer::sync::resolve_updates(&instance, &status.extra, &library);
    if updates.is_empty() {
        // Caller still gets a fresh snapshot back; the chip strip
        // re-renders from it idempotently.
        return Ok(instance);
    }
    let updated = crate::core::printer::instance_registry::mutate_instance(
        &instance_id,
        |inst| {
            for u in &updates {
                if let Some(ext) = inst.extruders.get_mut(u.extruder_idx) {
                    if let Some(slot) = ext.slots.get_mut(u.slot_idx) {
                        slot.filament_identity = u.filament_identity.clone();
                        slot.color = Some(u.color.clone());
                    }
                }
            }
            Ok(())
        },
    )
    .map_err(|e| e.to_string())?;
    use tauri::Emitter;
    if let Err(e) = window.emit("printer:instance_changed", &updated.id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
    Ok(updated)
}
