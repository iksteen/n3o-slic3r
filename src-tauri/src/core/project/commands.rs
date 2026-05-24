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
fn emit_all(window: &Window, events: &[SceneEvent]) {
    for event in events {
        if let Err(e) = window.emit(event.name(), event) {
            tracing::warn!(
                event = event.name(),
                error = %e,
                "project event emit failed (frontend disconnected?)",
            );
        }
    }
}

/// Set a plate's composition order (FR-MP-7). Auto-shifts the
/// remaining plates so `composition_order` stays a dense
/// `[1..plates.len()]` sequence. Emits one
/// `PlateMetadataChanged` per affected plate (the moved plate +
/// every plate whose order shifted to make room).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_set_plate_composition_order(
    plate_id: PlateId,
    order: u32,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_plate_composition_order(plate_id, order)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Set (upsert) a `material → slot` mapping on a plate (PR-S-7).
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

// ---- Save / load (PR-5-8) ------------------------------------------

/// Save the in-memory project to `path` as an n3o-slic3r `.3mf`.
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
    let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    format::write_project(&p, std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(
        &window,
        &[SceneEvent::ProjectSaved { path: path.clone() }],
    );
    Ok(())
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
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    format::write_project(&p, std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    p.source_path = Some(PathBuf::from(&path));
    drop(p);
    emit_all(
        &window,
        &[SceneEvent::ProjectSaved { path: path.clone() }],
    );
    Ok(())
}

/// Load a project file from `path`, **replacing** the in-memory
/// project wholesale. Emits `project:loaded { path }` so the
/// frontend mirror can throw out its cached scene state and
/// re-sync via `scene_snapshot`.
///
/// Returns the loaded project so the caller doesn't have to chain
/// a separate read; the frontend can render immediately from the
/// return value while the mirror catches up via the event.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_load(
    path: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<Project, String> {
    let loaded = format::read_project(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    let returned = loaded.clone();
    {
        let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
        *p = loaded;
    }
    emit_all(
        &window,
        &[SceneEvent::ProjectLoaded { path: path.clone() }],
    );
    Ok(returned)
}

// ---- Autosave (PR-5-10) --------------------------------------------

/// Start the autosave worker if not already running. Idempotent.
/// Uses [`autosave::default_autosave_dir`] for the on-disk
/// location and [`autosave::DEFAULT_INTERVAL`] (30 s) for the
/// tick rate. Failures to create the autosave directory surface
/// as an Err.
#[tauri::command]
#[tracing::instrument(skip(state, handle))]
pub fn project_autosave_enable(
    state: State<Arc<Mutex<Project>>>,
    handle: State<AutosaveHandle>,
) -> Result<(), String> {
    let dir = autosave::default_autosave_dir();
    let config = AutosaveConfig::new(dir);
    let project = (*state).clone();
    handle.start(project, config).map_err(|e| e.to_string())
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
