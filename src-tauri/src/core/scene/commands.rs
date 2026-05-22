//! Tauri commands that drive the scene state (PR-2-2).
//!
//! Each command takes a `Window` + `State<Mutex<SceneState>>`, locks
//! the state, calls a pure `SceneState` mutation method, emits the
//! returned events via `Window::emit`, and returns the result. Tests
//! for the *behavior* live in `state.rs` against the pure methods;
//! this file only validates the Tauri plumbing.

use super::events::{SceneEvent, SceneOpError, SelectMode};
use super::state::{CameraState, GizmoState, MeshId, ObjectId, SceneState};
use super::transform::Transform;
use serde::Deserialize;
use std::sync::Mutex;
use tauri::{Emitter, State, Window};

/// Emit each event on the given window. Errors are dropped — a
/// dropped frontend connection shouldn't fail a command.
fn emit_all(window: &Window, events: &[SceneEvent]) {
    for event in events {
        if let Err(e) = window.emit(event.name(), event) {
            tracing::warn!(
                event = event.name(),
                error = %e,
                "scene event emit failed (frontend disconnected?)"
            );
        }
    }
}

/// Snapshot of the full scene state. Frontend calls this on
/// startup / reconnect to rebuild its local mirror from scratch.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn scene_snapshot(state: State<Mutex<SceneState>>) -> Result<SceneState, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    Ok(s.clone())
}

/// Replace / add / toggle the selection.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_select(
    ids: Vec<ObjectId>,
    mode: SelectMode,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.select(&ids, mode);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Clear the selection.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_deselect(
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.deselect_all();
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vec3Json {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl From<Vec3Json> for glam::Vec3 {
    fn from(v: Vec3Json) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_translate(
    id: ObjectId,
    delta: Vec3Json,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .translate_object(id, delta.into())
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_rotate(
    id: ObjectId,
    axis: Vec3Json,
    radians: f32,
    pivot: Option<Vec3Json>,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .rotate_object(id, axis.into(), radians, pivot.map(Into::into))
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_scale(
    id: ObjectId,
    factor: Vec3Json,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .scale_object(id, factor.into())
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_set_transform(
    id: ObjectId,
    transform: Transform,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .set_object_transform(id, transform)
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_delete(
    ids: Vec<ObjectId>,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.delete_objects(&ids);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_duplicate(
    id: ObjectId,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<ObjectId, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (new_id, events) = s.duplicate_object(id).map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(new_id)
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_gizmo_set(
    gizmo: GizmoState,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_gizmo(gizmo);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_camera_set(
    camera: CameraState,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_camera(camera);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

fn op_err_to_string(e: SceneOpError) -> String {
    e.to_string()
}

// Mesh-loading commands (PR-2-3 / PR-2-4 own the actual loaders;
// scene_load_mesh is wired here so PR-2-2 ships the surface even
// when the loaders haven't landed). Placeholder until PR-2-3 plugs
// in real STL/OBJ paths — for now, the only producers of `Mesh`
// values are the unit tests and (eventually) PR-2-7's procedural
// primitives.
#[tauri::command]
#[tracing::instrument(skip(state, window, mesh))]
pub fn scene_load_mesh(
    mesh: super::state::Mesh,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(MeshId, ObjectId), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (mesh_id, obj_id, events) = s.load_mesh(mesh);
    drop(s);
    emit_all(&window, &events);
    Ok((mesh_id, obj_id))
}
