//! Tauri commands that mutate project-level state.
//!
//! Sibling to `core::scene::commands` — same wiring pattern (lock,
//! mutate, emit) but scoped to `Project`-level concerns: plate
//! metadata, project-tier overrides, file metadata, eventually
//! save/load. Scene-graph mutations (translate object, select,
//! gizmo) live in `core::scene::commands`.
//!
//! Both live behind the same `Mutex<Project>` Tauri state — the
//! split is naming-only, picked to keep each file focused as Phase
//! 5/6/7 commands accumulate.

use std::sync::{Arc, Mutex};

use tauri::{Emitter, State, Window};

use std::path::PathBuf;

use super::autosave::{self, AutosaveConfig, AutosaveEntry, AutosaveHandle};
use super::format;
use super::PlateId;
use super::Project;
use crate::core::scene::events::SceneEvent;

/// Emit each event on the given window. Errors are dropped — a
/// dropped frontend connection shouldn't fail a command.
pub(crate) fn emit_all(window: &Window, events: &[SceneEvent]) {
    for event in events {
        if let Err(e) = window.emit(event.name(), event) {
            tracing::warn!(
                event = event.name(),
                error = %e,
                "project event emit failed (frontend disconnected?)",
            );
        }
    }
    super::dirty::track(window, events);
    super::history::track(window, events);
}

/// Set (upsert) a `material → slot` mapping on a plate.
/// The slot reference is validated against the plate's bound
/// PrinterInstance; out-of-range indices error with
/// `InvalidPlateMetadata`.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_set_material_slot(
    plate_id: PlateId,
    model_material: u8,
    slot: crate::core::printer::SlotRef,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_material_slot(plate_id, model_material, slot)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Drop a plate's `material → slot` entry. Silent no-op when the
/// material has no current mapping.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_clear_material_slot(
    plate_id: PlateId,
    model_material: u8,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .clear_material_slot(plate_id, model_material)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Resolve a plate's cascade for the settings panel: compose the bound
/// instance's fragments against the plate's effective process
/// (`plate.quality_profile` overriding the instance default) and resolve
/// each key, tagging it with the layer it came from. **No** override tiers
/// are folded in — the panel draws project/object rows from its own maps;
/// these are the fragment-resolved values that fill the cascade rows
/// (Printer / Build plate / Filament / Profile). Returns an empty map for
/// an unbound plate.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn plate_cascade_resolve(
    plate_id: PlateId,
    state: State<Arc<Mutex<Project>>>,
) -> Result<super::resolve::PlateResolvedJson, String> {
    let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    super::resolve::resolve_plate_cascade(&p, plate_id)
}

/// Trace why a resolved key holds its value on a plate: which tier (cascade /
/// user / project) won, the cascade fallback if an override took over, and
/// the matching authored rules. Powers the settings panel's "why is X = Y"
/// affordance. `None` when the plate is unbound or the key isn't resolved.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn plate_cascade_trace(
    plate_id: PlateId,
    key: String,
    state: State<Arc<Mutex<Project>>>,
) -> Result<Option<crate::core::cascade::Trace>, String> {
    let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    match super::resolve::resolve_plate_with_tiers(&p, plate_id)? {
        Some(resolved) => Ok(crate::core::cascade::trace(&resolved, &key)),
        None => Ok(None),
    }
}

/// Set (or clear, with `None`) a plate's process/quality profile —
/// the bundled process-fragment slug this plate resolves + slices
/// against, overriding the bound instance's default. Validated against
/// the printer's bundled processes; emits `PlateMetadataChanged`.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_set_plate_quality_profile(
    plate_id: PlateId,
    quality_profile: Option<String>,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_plate_quality_profile(plate_id, quality_profile)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Viewport-managed per-plate placement keys: the wipe/prime-tower position
/// the user drags in the 3D view writes these into `project_overrides`
/// (see `viewport/WgpuViewport.tsx`). They're Process-bucket, but they're a
/// per-plate placement, not a reusable quality setting — so the stamp
/// excludes them (dragging the tower must never bake into the shared
/// quality profile).
const STAMP_EXCLUDED_KEYS: &[&str] = &["wipe_tower_x", "wipe_tower_y"];

/// The stampable subset of a plate's `project_overrides`: Process-bucket keys
/// that aren't viewport-managed placement ([`STAMP_EXCLUDED_KEYS`]). Each is
/// returned as `Some(value)` (the stamp form). A dragged tower
/// (`wipe_tower_x/y`) and any non-process key (filament/printer/metadata) are
/// dropped, so they never bake into the shared quality profile.
fn stampable_process_overrides(
    overrides: &std::collections::HashMap<String, String>,
) -> std::collections::BTreeMap<String, Option<String>> {
    overrides
        .iter()
        .filter(|(k, _)| {
            slic3r_ffi::bucket_of(k) == Some(slic3r_ffi::OptBucket::Process)
                && !STAMP_EXCLUDED_KEYS.contains(&k.as_str())
        })
        .map(|(k, v)| (k.clone(), Some(v.clone())))
        .collect()
}

/// Stamp the active plate's current quality edits onto its selected process
/// profile as a per-user override — the "Save" beside the Quality picker.
///
/// Takes the stampable Process-bucket keys in the plate's `project_overrides`
/// (the tier the panel writes quality edits to — minus the viewport-managed
/// placement keys in [`STAMP_EXCLUDED_KEYS`]) and merges them onto the
/// selected profile's stamped override (keyed by the printer + the bundled
/// process slug). With `clear`, those keys are then also removed from the
/// plate tier, so the diff lives *only* on the reusable profile (save then
/// clear); otherwise they stay on the plate too (a plain save that copies
/// them onto the profile). Effective values are unchanged either way; `clear`
/// only decides whether the transient per-plate edits are tidied up.
/// Non-process and excluded keys are always left on the plate.
///
/// A no-op (still `Ok`) when the plate is unbound or carries no stampable
/// edits. The selected profile is the plate's `quality_profile` when set,
/// else the bound instance's default.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn user_process_stamp(
    plate_id: PlateId,
    clear: bool,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id() else {
        return Ok(()); // unbound — nothing to stamp
    };
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    let printer = instance.printer_fragment_slug.clone();
    // The selected process: the plate's own when set, else the instance default.
    let base = plate
        .quality_profile
        .clone()
        .unwrap_or_else(|| instance.quality_profile.clone());

    // The stampable process-bucket subset of the plate's project-tier
    // overrides — the current quality diff to stamp (excluding the
    // viewport-managed tower-placement keys).
    let stamped = stampable_process_overrides(&plate.project_overrides);
    if stamped.is_empty() {
        return Ok(()); // no quality edits to save
    }

    crate::core::process::library::stamp(&printer, &base, stamped.clone());

    // With `clear` (the ⌘/Ctrl modifier), tidy the stamped keys off the plate
    // tier — they now live solely on the profile. A plain save leaves them.
    let mut events = Vec::new();
    if clear {
        for key in stamped.keys() {
            events.extend(
                p.project_override_clear(plate_id, key)
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    drop(p);
    emit_all(&window, &events);
    crate::core::process::emit_changed(&window);
    Ok(())
}

/// Save the active plate's current quality settings as a new **named** custom
/// profile — the "Duplicate" beside the Quality picker. The custom profile
/// inherits the selected profile's base fragment + its overrides, with the
/// plate's current stampable quality edits merged on top, under `name`. The
/// plate is then switched onto the new profile. With `clear` (the ⌘/Ctrl
/// modifier), the merged edits are also removed from the plate tier (save
/// then clear); otherwise they stay. Returns the new profile's id.
///
/// Errors on an unbound plate or a blank name.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn user_process_duplicate(
    plate_id: PlateId,
    name: String,
    clear: bool,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<String, String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("a profile name is required".into());
    }
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let instance_id = plate
        .printer_instance_id()
        .ok_or("plate is not bound to a printer")?;
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    let printer = instance.printer_fragment_slug.clone();
    let selected = plate
        .quality_profile
        .clone()
        .unwrap_or_else(|| instance.quality_profile.clone());

    // Inherit the selected profile's base fragment + overrides (a bundled
    // slug inherits itself with no overrides; a stamped/custom one its base +
    // saved overrides), then merge the plate's current quality edits.
    let (base, mut overrides) = match crate::core::process::library::lookup(&printer, &selected) {
        Some(up) => (up.base, up.overrides),
        None => (selected, std::collections::BTreeMap::new()),
    };
    let edits = stampable_process_overrides(&plate.project_overrides);
    for (k, v) in &edits {
        if let Some(v) = v {
            overrides.insert(k.clone(), v.clone());
        }
    }

    let created = crate::core::process::library::create_custom(&printer, &base, name, overrides);

    // Switch the plate onto the new profile.
    let mut events = p
        .set_plate_quality_profile(plate_id, Some(created.id.clone()))
        .map_err(|e| e.to_string())?;
    if clear {
        for key in edits.keys() {
            events.extend(
                p.project_override_clear(plate_id, key)
                    .map_err(|e| e.to_string())?,
            );
        }
    }
    drop(p);
    emit_all(&window, &events);
    crate::core::process::emit_changed(&window);
    Ok(created.id)
}

/// Write a removed profile's `overrides` onto the plate's project (plate)
/// tier, so reverting/deleting can *keep* the settings as project overrides
/// rather than discarding them. Returns the emitted events.
fn apply_overrides_to_plate(
    p: &mut Project,
    plate_id: PlateId,
    overrides: std::collections::BTreeMap<String, String>,
) -> Result<Vec<SceneEvent>, String> {
    let mut events = Vec::new();
    for (k, v) in overrides {
        events.extend(
            p.project_override_set(plate_id, k, v)
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(events)
}

/// Resolve the bound printer slug + the plate's selected process slug (its own
/// `quality_profile`, else the instance default), or `Ok(None)` when the plate
/// is unbound.
fn plate_printer_and_process(
    p: &Project,
    plate_id: PlateId,
) -> Result<Option<(String, String)>, String> {
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id() else {
        return Ok(None);
    };
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    let printer = instance.printer_fragment_slug.clone();
    let process = plate
        .quality_profile
        .clone()
        .unwrap_or_else(|| instance.quality_profile.clone());
    Ok(Some((printer, process)))
}

/// Discard a stamp-in-place profile's user overrides — back to pristine
/// bundled — for the plate's selected profile. The "Revert" beside the
/// Quality picker. With `apply`, the profile's settings are first written onto
/// the plate's project tier (so they live on as project overrides instead of
/// being lost). No-op when the plate is unbound or its profile is pristine.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn user_process_revert(
    plate_id: PlateId,
    apply: bool,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let Some((printer, selected)) = plate_printer_and_process(&p, plate_id)? else {
        return Ok(());
    };
    let Some(profile) = crate::core::process::library::lookup(&printer, &selected) else {
        return Ok(()); // pristine — nothing to revert
    };
    crate::core::process::library::remove(&printer, &selected);
    let mut events = Vec::new();
    if apply {
        events = apply_overrides_to_plate(&mut p, plate_id, profile.overrides)?;
    }
    drop(p);
    emit_all(&window, &events);
    crate::core::process::emit_changed(&window);
    Ok(())
}

/// Delete the active plate's selected **named custom** quality profile and
/// switch the plate back to its default (the bound instance's profile) — the
/// "Delete" the Revert button becomes when a custom profile is selected. With
/// `apply`, the custom profile's settings are first written onto the plate's
/// project tier (so they live on as project overrides over the default
/// profile, instead of being lost). A no-op (still `Ok`) when the plate is
/// unbound or its selected profile isn't a custom one.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn user_process_delete(
    plate_id: PlateId,
    apply: bool,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let Some((printer, selected)) = plate_printer_and_process(&p, plate_id)? else {
        return Ok(());
    };
    let profile = match crate::core::process::library::lookup(&printer, &selected) {
        Some(up) if up.id != up.base => up,
        _ => return Ok(()), // not a custom profile — nothing to delete
    };
    crate::core::process::library::remove(&printer, &selected);
    // Back to the default profile (inherit the instance's).
    let mut events = p
        .set_plate_quality_profile(plate_id, None)
        .map_err(|e| e.to_string())?;
    if apply {
        events.extend(apply_overrides_to_plate(&mut p, plate_id, profile.overrides)?);
    }
    drop(p);
    emit_all(&window, &events);
    crate::core::process::emit_changed(&window);
    Ok(())
}

// ---- Save / load ------------------------------------------

/// Save the in-memory project to `path` as an n3o-slic3r `.n3o` file.
/// Overwrites the file if it exists. The project's `source_path`
/// is **not** updated; use [`project_save_as`] when the user
/// chooses a new path via Save As.
///
/// Emits `project:saved { path }` after the write completes so the
/// UI can refresh the recent-files list / window-title indicator.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_save(
    path: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    // Clone under the lock, then write off-lock: the zip-to-disk is slow
    // (geometry blobs) and must not block scene mutations. Mirrors autosave.
    let snapshot = {
        let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
        p.clone()
    };
    format::write_project(&snapshot, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    // Emit (clears the dirty flag, gating the autosave worker off) before
    // dropping the now-redundant recovery file.
    emit_all(&window, &[SceneEvent::ProjectSaved { path: path.clone() }]);
    drop_recovery_after_save(&snapshot.uuid.to_string());
    Ok(())
}

/// Best-effort: remove the project's autosave recovery file once it has
/// been saved — the on-disk save is now canonical, so the crash snapshot
/// is stale. Logged, never fatal to the save.
fn drop_recovery_after_save(uuid: &str) {
    let dir = autosave::default_autosave_dir();
    if let Err(e) = autosave::drop_autosave(&dir, uuid) {
        tracing::warn!(error = %e, uuid, "failed to drop autosave recovery after save");
    }
}

/// Save the in-memory project to `path` AND update its
/// `source_path` so subsequent `project_save` calls write here.
/// Use when the user picks a new file via Save As; for the
/// vanilla Save flow use [`project_save`].
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_save_as(
    path: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    // Set source_path on the live project, clone under the lock, then write
    // off-lock (see project_save — the zip-to-disk must not hold the mutex).
    let snapshot = {
        let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
        p.source_path = Some(PathBuf::from(&path));
        p.clone()
    };
    format::write_project(&snapshot, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    emit_all(&window, &[SceneEvent::ProjectSaved { path: path.clone() }]);
    drop_recovery_after_save(&snapshot.uuid.to_string());
    Ok(())
}

/// Load a project file from `path`, transparently importing a foreign
/// OrcaSlicer / Bambu Studio project (no `n3o_project.json`) instead of
/// erroring — then `Some(report)` carries the import summary. Pure: builds the
/// `Project` but does **not** swap it into state or emit; the app-shell
/// `project_io::project_load` command does the wholesale replace (which also
/// drops the renderer's GPU mesh cache — a thing `core` deliberately can't do,
/// per AD-8).
pub fn load_or_import(
    path: &std::path::Path,
) -> Result<(Project, Option<crate::core::orca_import::ImportReport>), String> {
    match format::read_project(path) {
        Ok(p) => Ok((p, None)),
        Err(format::ProjectIoError::ForeignProject { .. }) => {
            let (mut project, report) =
                crate::core::orca_import::import(path).map_err(|e| format!("import: {e}"))?;
            // A foreign .3mf import becomes a *native* project — point its save
            // target at the matching `.n3o` name so the app reflects that (the
            // user saves a native project, never back to the foreign 3mf).
            project.source_path = Some(path.with_extension("n3o"));
            Ok((project, Some(report)))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// A fresh, empty default project bound to the user's last-selected printer.
/// Pure: the app-shell `project_io::project_new` command swaps it into state +
/// emits. The new project has `source_path = None` ("Untitled"); the previous
/// project's autosave file is keyed by its own uuid and survives.
pub fn fresh_project() -> Project {
    let preferred = crate::core::config::load().defaults.printer_instance;
    Project::with_preferred_printer(preferred.as_deref())
}

// ---- Autosave --------------------------------------------

/// Start the autosave worker if not already running. Idempotent.
/// Uses [`autosave::default_autosave_dir`] for the on-disk
/// location and [`autosave::DEFAULT_INTERVAL`] (30 s) for the
/// tick rate. Failures to create the autosave directory surface
/// as an Err.
#[tauri::command]
#[tracing::instrument(skip(state, dirty, handle))]
pub fn project_autosave_enable(
    state: State<Arc<Mutex<Project>>>,
    dirty: State<Arc<super::dirty::DirtyTracker>>,
    handle: State<AutosaveHandle>,
) -> Result<(), String> {
    let dir = autosave::default_autosave_dir();
    let config = AutosaveConfig::new(dir);
    let project = (*state).clone();
    handle
        .start(project, (*dirty).clone(), config)
        .map_err(|e| e.to_string())
}

/// Whether the project has unsaved edits — the backend-authoritative
/// dirty flag. The frontend reads this once on mount, then tracks
/// `project:dirty_changed` events. Source of truth for the title-bar
/// unsaved marker.
#[tauri::command]
pub fn project_is_dirty(dirty: State<Arc<super::dirty::DirtyTracker>>) -> bool {
    dirty.is_dirty()
}

// ---- Undo / redo -----------------------------------------

/// Current can-undo / can-redo state for the menu + button enablement.
/// The frontend reads this on mount, then tracks `project:history_changed`.
#[derive(serde::Serialize)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[tauri::command]
pub fn project_history_state(
    history: State<Arc<Mutex<super::history::UndoHistory>>>,
) -> HistoryState {
    let h = history.lock().expect("history poisoned");
    HistoryState {
        can_undo: h.can_undo(),
        can_redo: h.can_redo(),
    }
}

/// Undo the last edit. No-op (returns `false`) when there's nothing to
/// undo. Restores the prior snapshot, resyncs the renderer
/// (`project:restored`), and re-dirties the project.
#[tauri::command]
pub fn project_undo(
    window: Window,
    state: State<Arc<Mutex<Project>>>,
    history: State<Arc<Mutex<super::history::UndoHistory>>>,
) -> bool {
    super::history::apply_step(&window, &state, &history, false)
}

/// Redo the last undone edit. No-op (`false`) when the redo branch is empty.
#[tauri::command]
pub fn project_redo(
    window: Window,
    state: State<Arc<Mutex<Project>>>,
    history: State<Arc<Mutex<super::history::UndoHistory>>>,
) -> bool {
    super::history::apply_step(&window, &state, &history, true)
}

/// Stop the autosave worker. Idempotent — calling when the
/// worker isn't running is a silent no-op.
#[tauri::command]
#[tracing::instrument(skip(handle))]
pub fn project_autosave_disable(handle: State<AutosaveHandle>) -> Result<(), String> {
    handle.stop();
    Ok(())
}

/// List recoverable autosave files in the default autosave
/// directory. Returns entries newest-first. The frontend's
/// recovery dialog consumes this on app startup.
#[tauri::command]
#[tracing::instrument]
pub fn project_autosave_list() -> Result<Vec<AutosaveEntry>, String> {
    let dir = autosave::default_autosave_dir();
    autosave::scan_recoveries(&dir).map_err(|e| e.to_string())
}

/// Delete the autosave file for `uuid`. Wires the recovery
/// dialog's "Discard" button. Silent no-op when the file isn't
/// present (idempotent).
#[tauri::command]
#[tracing::instrument]
pub fn project_autosave_drop(uuid: String) -> Result<(), String> {
    let dir = autosave::default_autosave_dir();
    autosave::drop_autosave(&dir, &uuid).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn stamp_excludes_tower_placement_and_nonprocess_keys() {
        let _ = slic3r_ffi::init(None, 3); // bucket_of needs the schema
        let mut o = HashMap::new();
        o.insert("layer_height".to_owned(), "0.28".to_owned()); // process — kept
        o.insert("sparse_infill_density".to_owned(), "25%".to_owned()); // process — kept
        o.insert("wipe_tower_x".to_owned(), "42".to_owned()); // placement — excluded
        o.insert("wipe_tower_y".to_owned(), "130".to_owned()); // placement — excluded
        o.insert("nozzle_diameter".to_owned(), "0.4".to_owned()); // printer bucket — excluded
        let s = stampable_process_overrides(&o);
        assert!(s.contains_key("layer_height"));
        assert!(s.contains_key("sparse_infill_density"));
        assert!(!s.contains_key("wipe_tower_x"), "dragged tower must not stamp");
        assert!(!s.contains_key("wipe_tower_y"), "dragged tower must not stamp");
        assert!(!s.contains_key("nozzle_diameter"), "non-process key must not stamp");
    }

    #[test]
    fn apply_overrides_writes_them_to_the_plate_project_tier() {
        // The ⌘/Ctrl path of revert/delete preserves a removed profile's
        // settings by writing them onto the plate's project tier.
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        let mut ov = std::collections::BTreeMap::new();
        ov.insert("layer_height".to_owned(), "0.28".to_owned());
        ov.insert("outer_wall_speed".to_owned(), "60".to_owned());
        let events =
            apply_overrides_to_plate(&mut project, plate_id, ov).expect("apply succeeds");
        assert!(!events.is_empty(), "emits a ProjectOverridesChanged");
        let plate = project.plate(plate_id).expect("plate");
        assert_eq!(
            plate.project_overrides.get("layer_height").map(String::as_str),
            Some("0.28"),
        );
        assert_eq!(
            plate.project_overrides.get("outer_wall_speed").map(String::as_str),
            Some("60"),
        );
    }
}
