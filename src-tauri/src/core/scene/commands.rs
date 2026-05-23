//! Tauri commands that drive the scene state (PR-2-2).
//!
//! Each command takes a `Window` + `State<Mutex<SceneState>>`, locks
//! the state, calls a pure `SceneState` mutation method, emits the
//! returned events via `Window::emit`, and returns the result. Tests
//! for the *behavior* live in `state.rs` against the pure methods;
//! this file only validates the Tauri plumbing.

use super::events::{MirrorAxis, SceneEvent, SceneOpError, SelectMode};
use super::bed::BedMesh;
use super::state::{
    ActivePlate, CameraState, ExclusionZone, GizmoState, MeshHeader, MeshId, ObjectId,
    SceneObject, SceneState,
};
use crate::core::printer::profile::PrinterProfile;
use super::transform::Transform;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::ipc::Response;
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

/// JSON-friendly snapshot of the scene. Mesh buffers are *not*
/// included — only their headers; the frontend fetches the buffers
/// per-mesh via `scene_mesh_buffers` (see PR-2-2's mesh-transport
/// refactor).
#[derive(Debug, Clone, Serialize)]
pub struct SceneSnapshot {
    pub meshes: Vec<MeshHeader>,
    pub objects: Vec<SceneObject>,
    pub selection: Vec<ObjectId>,
    pub camera: CameraState,
    pub gizmo: GizmoState,
    pub plate: Option<ActivePlate>,
    pub exclusion_zones: Vec<ExclusionZone>,
    /// Active bed visualization + bounds. `None` until the user
    /// selects a printer.
    pub bed: Option<BedMesh>,
}

/// Snapshot of the scene state. Frontend calls this on startup /
/// reconnect to rebuild its local mirror from scratch. Heavy mesh
/// buffers are *not* included here — the renderer follows up with
/// `scene_mesh_buffers(mesh_id)` per mesh to fetch the binary data.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn scene_snapshot(state: State<Mutex<SceneState>>) -> Result<SceneSnapshot, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let meshes = s.meshes.values().map(|m| m.header()).collect();
    let objects = s.objects.values().cloned().collect();
    let selection: Vec<ObjectId> = {
        let mut v: Vec<ObjectId> = s.selection.iter().copied().collect();
        v.sort();
        v
    };
    let _ = HashSet::<ObjectId>::new(); // silence unused-import on certain feature builds
    Ok(SceneSnapshot {
        meshes,
        objects,
        selection,
        camera: s.camera.clone(),
        gizmo: s.gizmo.clone(),
        plate: s.plate.clone(),
        exclusion_zones: s.exclusion_zones.clone(),
        bed: s.bed.clone(),
    })
}

/// Install the active printer's bed visualization + bounds. Pass
/// `None` to clear (project closed / no printer selected).
#[tauri::command]
#[tracing::instrument(skip(state, window, printer))]
pub fn scene_set_active_printer(
    printer: Option<PrinterProfile>,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_active_printer(printer.as_ref());
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Install the bundled Bambu A1 mini profile as the active printer.
///
/// **Phase 2 bootstrapping** — gives the viewport something to
/// render before Phase 5 builds a real printer-selection UI. The
/// profile is hardcoded inline rather than read from disk so the
/// command works regardless of the runtime working directory; Phase
/// 5 will replace this with a profile registry that loads from
/// `profiles/printers/*.toml` and lets users pick + override.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_default_printer(
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    use crate::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
    let printer = PrinterProfile {
        model: "Bambu A1 mini".into(),
        slot_count: 4,
        supported_build_plates: vec![
            "Cool".into(),
            "Textured PEI".into(),
            "Smooth PEI".into(),
            "Engineering".into(),
            "SuperTack".into(),
        ],
        toolheads: vec![Toolhead {
            nozzle_diameter: 0.4,
            hotend_type: "stainless_steel".into(),
            max_temp: 300.0,
            slot_indices: vec![0, 1, 2, 3],
        }],
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [180.0, 180.0, 180.0],
        },
        exclusion_zones: vec![],
    };
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_active_printer(Some(&printer));
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument]
pub fn library_primitives() -> Vec<super::library::PrimitiveDescriptor> {
    super::library::list_primitives()
}

#[tauri::command]
#[tracing::instrument(skip(resources_root))]
pub fn library_calibration(
    printer_model: String,
    resources_root: String,
) -> Vec<super::library::CalibrationDescriptor> {
    super::library::list_calibration(&printer_model, std::path::Path::new(&resources_root))
}

#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn library_imported(
    state: State<Mutex<SceneState>>,
) -> Result<Vec<super::library::ImportedDescriptor>, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    Ok(super::library::list_imported(&s))
}

/// Add a procedural primitive to the scene at plate origin. Mesh
/// data is deduplicated within the scene — re-clicking "Add cube"
/// with the same parameters reuses the existing MeshId.
/// Greedy auto-arrange the current scene's visible objects onto
/// the active plate. No-op (returns empty placed/un_placed) when
/// no printer is active.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_auto_arrange(
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<Vec<ObjectId>, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let Some(bed) = s.bed.clone() else {
        return Ok(vec![]);
    };
    let plan = super::arrange::plan_arrangement(&s, &bed);
    let (mut events, un_placed) = super::arrange::apply_arrangement(&mut s, plan);
    drop(s);
    if !un_placed.is_empty() {
        events.push(SceneEvent::AutoArrangeOverflow {
            un_placed: un_placed.clone(),
        });
    }
    emit_all(&window, &events);
    Ok(un_placed)
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_add_from_primitive(
    kind: super::primitives::PrimitiveKind,
    params: super::primitives::PrimitiveParams,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(MeshId, ObjectId), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (mesh_id, obj_id, events) = s.add_from_primitive(kind, params);
    drop(s);
    emit_all(&window, &events);
    Ok((mesh_id, obj_id))
}

/// Return the binary vertex/normal/index buffers for one mesh.
/// Sequential layout: `[vertices_f32 ...][normals_f32 ...][indices_u32 ...]`
/// in little-endian. Lengths derive from the matching `MeshHeader`.
/// Sent as a binary `Response` to skip the JSON-array-of-floats
/// stringification entirely (47 MB STL → 36 MB binary, vs ~100 MB
/// JSON).
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn scene_mesh_buffers(
    mesh_id: MeshId,
    state: State<Mutex<SceneState>>,
) -> Result<Response, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let mesh = s
        .meshes
        .get(&mesh_id)
        .ok_or_else(|| format!("unknown mesh id {mesh_id:?}"))?;
    Ok(Response::new(mesh.pack_buffers()))
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
pub fn scene_object_mirror(
    id: ObjectId,
    axis: MirrorAxis,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.mirror_object(id, axis).map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_lay_flat(
    id: ObjectId,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.lay_flat_object(id).map_err(op_err_to_string)?;
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

/// Load a mesh from a file path (STL or OBJ) and register it as a
/// scene object at origin. The path-based form is the only public
/// load surface — caller-built mesh data (PR-2-7's procedural
/// primitives) reaches the registry through the same path once
/// PR-2-7 lands a `library_*` command set that emits to the loader
/// pipeline.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_mesh_from_path(
    path: String,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<(MeshId, ObjectId), String> {
    let new_mesh = super::loaders::load_mesh_from_path(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (mesh_id, obj_id, events) = s.load_mesh(new_mesh);
    drop(s);
    emit_all(&window, &events);
    Ok((mesh_id, obj_id))
}

/// Summary of one `<build><item>` ingested from a 3MF, returned by
/// `scene_load_3mf` so the frontend can highlight what just appeared.
#[derive(Debug, Clone, Serialize)]
pub struct LoadedObject {
    pub mesh_id: MeshId,
    pub object_id: ObjectId,
    pub name: String,
    pub extruder_id: Option<u8>,
    pub plate_id: u32,
}

/// Outcome of loading a `.3mf` project: every object registered into
/// the scene plus the file's informational metadata.
#[derive(Debug, Clone, Serialize)]
pub struct LoadedProject {
    pub objects: Vec<LoadedObject>,
    pub printer_hint: Option<String>,
    pub file_metadata: std::collections::BTreeMap<String, String>,
    /// Raw `Metadata/project_settings.config` if the file carried it.
    /// Phase 5 (Settings UI) parses this; for now the frontend
    /// surfaces it as opaque diagnostic text.
    pub embedded_settings: Option<String>,
}

/// Load a `.3mf` *project* file into the scene. Each `<build><item>`
/// becomes a scene object at its file-supplied transform with the
/// per-part extruder assignment preserved; BBS/Orca metadata
/// extensions are honored where present.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_3mf(
    path: String,
    window: Window,
    state: State<Mutex<SceneState>>,
) -> Result<LoadedProject, String> {
    use super::loaders::threemf;
    use super::state::NewSceneObject;

    let project = threemf::load_3mf(std::path::Path::new(&path)).map_err(|e| e.to_string())?;

    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;

    // Register every mesh first, capturing the allocated MeshIds in
    // the same order as Project3mf.meshes so ProjectObject.mesh_idx
    // remains a valid index lookup. Collect MeshLoaded events from
    // the headers as we go.
    let mut all_events: Vec<SceneEvent> = Vec::new();
    let mut mesh_ids: Vec<MeshId> = Vec::with_capacity(project.meshes.len());
    for new_mesh in project.meshes {
        let mesh_id = s.register_mesh(new_mesh);
        let header = s.meshes.get(&mesh_id).unwrap().header();
        all_events.push(SceneEvent::MeshLoaded(header));
        mesh_ids.push(mesh_id);
    }

    let mut loaded = Vec::with_capacity(project.objects.len());
    for obj in project.objects {
        let mesh_id = mesh_ids[obj.mesh_idx];
        let object_id = s.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: obj.transform,
            name: obj.name.clone(),
            visible: true,
            extruder_id: obj.extruder_id,
            parent: None,
        });
        let obj_clone = s.objects.get(&object_id).unwrap().clone();
        all_events.push(SceneEvent::ObjectAdded(obj_clone));
        loaded.push(LoadedObject {
            mesh_id,
            object_id,
            name: obj.name,
            extruder_id: obj.extruder_id,
            plate_id: obj.plate_id,
        });
    }
    drop(s);

    emit_all(&window, &all_events);

    Ok(LoadedProject {
        objects: loaded,
        printer_hint: project.printer_hint,
        file_metadata: project.file_metadata,
        embedded_settings: project.embedded_settings,
    })
}
