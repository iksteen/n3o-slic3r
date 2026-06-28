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
