//! Printer instances and catalog.
//!
//! This module owns the user-binding state layered over immutable
//! vendor printer profiles: the [`instance`] model (nozzles, slot
//! bindings, connection config), its registry/library/storage, the
//! catalog facade over the bundled printer profiles, and the Tauri
//! command surface that creates, mutates, and queries instances.
//! Driver comms — the network send/control path to a physical printer
//! — live in `core/driver`, not here.

pub mod bambu;
pub mod capability;
pub mod instance;
/// Bundled A1 mini + U1 instance fixtures. Test-only — gated out of release
/// builds (see the `test-fixtures` feature in Cargo.toml) so the shipped
/// binary ships no fixtures and a missing storage root yields an empty
/// registry, not fabricated printers.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod instance_library;
pub mod instance_registry;
pub mod instance_storage;
pub mod profile;
pub mod registry;
pub mod snapmaker;
pub mod sync;

pub use capability::{capability_for_key, CapabilityPredicate};
pub use instance::{
    BedRef, ConnectionInfo, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance,
    SlotBinding, SlotRef,
};
#[cfg(any(test, feature = "test-fixtures"))]
pub use instance_library::{
    bundled_instances, instance_id_for_vendor_profile, BAMBI_ID, SNAPPY_ID,
};
pub use instance_registry::{
    create_instance, delete_instance, list_instances, lookup_instance, mutate_instance,
    set_extruder_nozzle_diameter, set_instance_ams_units, set_instance_bed,
    set_instance_connection, set_instance_display_name, set_instance_quality_profile,
    set_config_override, set_plugin_override, set_slot_color, set_slot_filament, update_instance,
    InstanceMutError,
    InstancePatch,
};
pub use profile::{BoundingBox, PrinterProfile, Toolhead};
pub use registry::{bundled_catalog, default_printer_identity, lookup, CatalogEntry};

use crate::core::profile_library::{
    list_filament_fragments, list_process_fragments, FilamentFragmentSummary,
    ProcessFragmentSummary,
};
use tauri::Emitter;

/// Emit `printer:instance_changed` for `id`, logging (not failing) on a
/// transport error. Mirrors `filament::emit_changed`; consumers re-fetch
/// the affected instance on this event.
fn emit_instance_changed(window: &tauri::Window, id: &str) {
    if let Err(e) = window.emit("printer:instance_changed", id) {
        tracing::warn!(error = %e, "printer:instance_changed emit failed");
    }
}

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
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: set (or clear, with `value = None`) a `plugin.<name>.*`
/// entry on the instance — the printer-instance tier of the plugin cascade.
/// Only `plugin.*` keys are accepted. Emits `printer:instance_changed`.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_plugin_override(
    id: String,
    key: String,
    value: Option<String>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    if !key.starts_with("plugin.") {
        return Err(format!("`{key}` is not a plugin override key"));
    }
    let updated = set_plugin_override(&id, key, value).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: set (or clear, with `value = None`) a machine-settings
/// override on the instance — the printer-instance tier for Printer-bucket
/// keys. Only keys libslic3r classifies as Printer-bucket are accepted, so
/// this can't be used to smuggle process/filament keys past the cascade.
/// Emits `printer:instance_changed`.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_config_override(
    id: String,
    key: String,
    value: Option<String>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    if slic3r_ffi::bucket_of(&key) != Some(slic3r_ffi::OptBucket::Printer) {
        return Err(format!("`{key}` is not a machine (printer-bucket) setting"));
    }
    let updated = set_config_override(&id, key, value).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: resolve a printer instance's cascade and return the
/// flat `key → value` map. The machine-settings panel uses it to show
/// each option's *resolved* base value (the printer fragment's machine
/// globals + any instance override), so a row reads the printer's real
/// configured value rather than libslic3r's compile-time default.
#[tauri::command]
#[tracing::instrument]
pub fn printer_instance_resolved_config(
    id: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    let inst =
        lookup_instance(&id).ok_or_else(|| format!("unknown printer instance `{id}`"))?;
    let resolved = crate::core::project::commands::resolve_instance_cascade(
        &inst,
        None,
        &std::collections::BTreeMap::new(),
    )?;
    Ok(resolved.into_iter().map(|(k, v)| (k, v.value)).collect())
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
    let inst =
        create_instance(&printer_identity, display_name, ams_units).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &inst.id);
    Ok(inst)
}

/// Tauri command: remove a `PrinterInstance`. The caller (frontend)
/// is responsible for unbinding any plates that referenced this
/// instance; the backend currently leaves them as dangling refs
/// (the slice gate refuses to run, the picker surfaces "unbound").
/// Emits `printer:instance_changed` so consumers re-list.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_delete(id: String, window: tauri::Window) -> Result<(), String> {
    delete_instance(&id).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &id);
    Ok(())
}

/// Tauri command: atomically rebind or unbind a set of plates and
/// then delete a `PrinterInstance`. Closes the partial-commit
/// window the old frontend orchestration had: a sequential or
/// parallel rebind loop followed by `delete_instance` left a
/// fragile gap where some plates had been moved while the delete
/// itself could still fail.
///
/// `fallback_instance_id` is `Some` when a fallback printer
/// exists (plates get rebound to it) and `None` for the
/// last-printer-delete flow (plates get unbound — their
/// `printer_instance_id` becomes `None` and the bed visualization
/// is cleared).
///
/// Single registry + project lock, single batch of scene events.
/// Plates not in `plate_ids` are untouched.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn printer_instance_delete_with_reassign(
    id: String,
    fallback_instance_id: Option<String>,
    plate_ids: Vec<crate::core::project::PlateId>,
    state: tauri::State<'_, std::sync::Arc<std::sync::Mutex<crate::core::project::Project>>>,
    window: tauri::Window,
) -> Result<(), String> {
    // Resolve the fallback's profile up-front so the per-plate
    // rebind doesn't have to repeat the catalog lookup. Lookup
    // failure surfaces as a typed error before any project state
    // mutates.
    let fallback = if let Some(fid) = fallback_instance_id.as_deref() {
        let inst =
            lookup_instance(fid).ok_or_else(|| format!("no printer instance with id `{fid}`"))?;
        let profile = lookup(&inst.vendor_profile_ref).ok_or_else(|| {
            format!(
                "printer instance `{fid}` references unknown vendor profile `{}`",
                inst.vendor_profile_ref,
            )
        })?;
        Some((fid.to_owned(), profile))
    } else {
        None
    };

    let mut all_events = Vec::new();
    {
        let mut project = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        for plate_id in plate_ids {
            let events = match &fallback {
                Some((fid, profile)) => project
                    .rebind_plate_printer(plate_id, fid.clone(), profile)
                    .map(|(_report, events)| events)
                    .map_err(|e| e.to_string())?,
                None => project
                    .unbind_plate_printer(plate_id)
                    .map_err(|e| e.to_string())?,
            };
            all_events.extend(events);
        }
    }
    delete_instance(&id).map_err(|e| e.to_string())?;
    crate::core::scene::commands::emit_all(&window, &all_events);
    emit_instance_changed(&window, &id);
    Ok(())
}

/// Tauri command: rename the instance. Trims whitespace and rejects
/// empty (mirrors `create_instance`'s validation). Emits
/// `printer:instance_changed` so consumers re-list.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_display_name(
    id: String,
    display_name: String,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_instance_display_name(&id, display_name).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: change the AMS-unit count on an AMS-style printer.
/// Re-derives slot topology + preserves existing bindings positionally;
/// shrinks drop overflow bindings with a `tracing::warn!`. Rejects
/// for toolchangers and for values above `profile.ams_max`.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_ams_units(
    id: String,
    ams_units: u32,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_instance_ams_units(&id, ams_units).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: atomic multi-field update. Applies the patch
/// under one registry lock (one persist + one
/// `printer:instance_changed` emit). The settings modal uses this
/// instead of issuing three sequential per-field IPCs (display
/// name + AMS units + connection) — collapses the partial-success
/// window where one mutator succeeded and a later one threw.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_update(
    id: String,
    patch: InstancePatch,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = update_instance(&id, patch).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: write (or clear) the instance's network connection
/// settings. Persisted via `instance_storage`; the reactive driver
/// manager (`useDriverConnections`) reconciles the live driver
/// registry off the resulting `printer:instance_changed` event.
/// Rejects a `ConnectionInfo` whose variant doesn't match the
/// printer's `driver_kind`.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_connection(
    id: String,
    connection: Option<ConnectionInfo>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_instance_connection(&id, connection).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
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
    diameter: String,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated =
        set_extruder_nozzle_diameter(&id, extruder_idx, diameter).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
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
    let updated = set_instance_bed(&id, bed_identity).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
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
    let updated = set_slot_color(&id, extruder_idx, slot_idx, color).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: bundled vendor filament fragments
/// (`profiles/<vendor>/filament/<slug>.toml`). Each entry
/// carries the slug (the wire `filament_identity` the slot panel
/// stores) plus a display label parsed out of the fragment's
/// `filament_settings_id` field, plus the base material type for
/// the picker swatch.
///
/// Drives the slot-binding panel's filament dropdown — what the
/// user actually picks from when binding a slot.
#[tauri::command]
pub fn filament_profile_list() -> Vec<FilamentFragmentSummary> {
    // Bundled filaments are edited in place: each keeps its identity, and
    // carries an `edited` flag when the user has an override profile for it
    // (which the picker surfaces as a Revert affordance).
    let edited: std::collections::HashSet<String> = crate::core::filament::library::list()
        .into_iter()
        .map(|f| f.base)
        .collect();
    list_filament_fragments()
        .into_iter()
        .map(|mut s| {
            s.edited = edited.contains(&s.identity);
            s
        })
        .collect()
}

/// Tauri command: change the process fragment ("Quality" picker)
/// selected for an instance. Drives the cascade re-resolve via the
/// `printer:instance_changed` event the bed and nozzle setters
/// already use.
#[tauri::command]
#[tracing::instrument(skip(window))]
pub fn printer_instance_set_quality_profile(
    id: String,
    quality_profile: String,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let updated = set_instance_quality_profile(&id, quality_profile).map_err(|e| e.to_string())?;
    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}

/// Tauri command: enumerate process fragments available for the
/// active (printer, installed-nozzle-set). Drives the Quality
/// picker chip in the settings panel — the frontend reads each
/// fragment's `[meta] available_for` (loaded at startup) and only
/// surfaces processes whose listing includes any installed nozzle.
///
/// `printer_fragment_slug` is the printer directory slug (e.g.
/// `"bambu-lab-a1-mini"`); `printer_model` is the human printer
/// name from `machine.toml` (e.g. `"Bambu Lab A1 mini"`) — the
/// metadata's `available_for` rows key off the latter, while the
/// on-disk fragments live under the former. Frontend already has
/// both from the active `PrinterProfile`.
///
/// `installed_nozzle_diameters` is the unique set of nozzle
/// diameters currently installed across the printer's extruders
/// (single-extruder printers send `["0.4"]`; a U1 toolchanger with
/// mixed nozzles sends `["0.4", "0.6"]`). A fragment matches when
/// any constituent nozzle of its `available_for` entry (split on
/// `+`) shares at least one diameter with the installed set —
/// composite profiles like `0.4+0.6` surface whenever any of their
/// nozzles is present.
#[tauri::command]
pub fn process_fragment_list(
    printer_fragment_slug: String,
    printer_model: String,
    installed_nozzle_diameters: Vec<String>,
) -> Vec<ProcessFragmentSummary> {
    list_process_fragments(
        &printer_fragment_slug,
        &printer_model,
        &installed_nozzle_diameters,
    )
}

/// Tauri command: pull the named printer's current spool loadout
/// from the live driver into the instance's SlotBindings.
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
    registry: tauri::State<'_, std::sync::Arc<crate::core::driver::registry::DriverRegistry>>,
    window: tauri::Window,
) -> Result<PrinterInstance, String> {
    let handle = registry
        .get(driver_id)
        .ok_or_else(|| format!("unknown driver id {}", driver_id.0))?;
    let status = { handle.read().await.status() };
    let library = list_filament_fragments();

    // Reconcile AMS-unit count + per-slot loadout in one atomic
    // mutation (single lock + persist). The topology/eligibility
    // policy lives in the sync module alongside the rest of the
    // driver-report translation.
    let updated =
        crate::core::printer::sync::apply_from_driver(&instance_id, &status.extra, &library)
            .map_err(|e| e.to_string())?;

    emit_instance_changed(&window, &updated.id);
    Ok(updated)
}
