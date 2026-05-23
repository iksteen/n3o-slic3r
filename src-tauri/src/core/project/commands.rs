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

use std::sync::Mutex;

use tauri::{Emitter, State, Window};

use std::path::PathBuf;

use super::binding::MaterialBinding;
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

/// Set a plate's cycle count (FR-MP-7). Validates against the
/// 1..=999 range declared in `PlateMetadata`; emits
/// `PlateMetadataChanged` so the tab-strip badge re-renders.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_set_plate_cycle_count(
    plate_id: PlateId,
    count: u32,
    window: Window,
    state: State<Mutex<Project>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_plate_cycle_count(plate_id, count)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
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
    state: State<Mutex<Project>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_plate_composition_order(plate_id, order)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Upsert a material binding on a plate (FR-MP-8). The caller
/// passes the resolved 1-based indices + the filament profile
/// identity loaded in the slot.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_set_material_binding(
    plate_id: PlateId,
    model_material: u8,
    physical_slot: u8,
    filament_identity: String,
    window: Window,
    state: State<Mutex<Project>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .set_material_binding(plate_id, model_material, physical_slot, filament_identity)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Drop a plate's binding for `model_material`. The model material
/// falls back to "use slot 1" at slice time per FR-MP-8.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_clear_material_binding(
    plate_id: PlateId,
    model_material: u8,
    window: Window,
    state: State<Mutex<Project>>,
) -> Result<(), String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let events = p
        .clear_material_binding(plate_id, model_material)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(())
}

/// Auto-bind every model material referenced by objects on the
/// plate to a sequential physical slot (1..=slot_count). Returns
/// the resulting binding list. Phase 5 stub of FR-FS-10's family-
/// aware heuristic — Phase 7c upgrades to "match by filament
/// family" once live slot state lands.
///
/// `slot_count` is caller-supplied: comes from the resolved
/// `PrinterProfile` for the plate's bound printer (no profile
/// registry yet — see PR-5-4).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_auto_bind_materials(
    plate_id: PlateId,
    slot_count: u8,
    window: Window,
    state: State<Mutex<Project>>,
) -> Result<Vec<MaterialBinding>, String> {
    let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    let (bindings, events) = p
        .auto_bind_materials(plate_id, slot_count)
        .map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &events);
    Ok(bindings)
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
    state: State<Mutex<Project>>,
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
    state: State<Mutex<Project>>,
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
    state: State<Mutex<Project>>,
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
