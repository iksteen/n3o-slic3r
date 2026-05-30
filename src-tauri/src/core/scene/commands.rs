//! Tauri commands that drive the scene side of [`Project`].
//!
//! Each command takes a `Window` + `State<Arc<Mutex<Project>>>`, locks
//! the state, calls a pure `Project` mutation method, emits the
//! returned events via `Window::emit`, and returns the result. Tests
//! for the *behavior* live in `core::project::mutation` against the
//! pure methods; this file only validates the Tauri plumbing.

use super::bed::BedMesh;
use super::events::{MirrorAxis, MoveReport, SceneEvent, SceneOpError, SelectMode};
use super::state::{ActivePlate, ExclusionZone, MeshHeader, MeshId, ObjectId, SceneObject};
use super::transform::Transform;
use crate::core::printer::profile::PrinterProfile;
use crate::core::project::{PlateId, Project};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tauri::ipc::Response;
use tauri::{Emitter, State, Window};

/// Emit each event on the given window. Errors are dropped — a
/// dropped frontend connection shouldn't fail a command.
pub(crate) fn emit_all(window: &Window, events: &[SceneEvent]) {
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

/// JSON-friendly snapshot of the entire project. The frontend
/// calls `scene_snapshot` on startup / reconnect to rebuild its
/// local mirror from scratch; subsequent updates arrive as scoped
/// `SceneEvent`s (each carrying its `plate_id`).
///
/// Mesh buffers are *not* included — only their headers; the
/// frontend fetches the buffers per-mesh via `scene_mesh_buffers`.
#[derive(Debug, Clone, Serialize)]
pub struct SceneSnapshot {
    /// Stable per-project identifier baked at project creation.
    pub project_uuid: String,
    /// Filesystem path the project was loaded from (or `None`
    /// for an unsaved in-memory project).
    pub source_path: Option<String>,
    /// User-tier cascade overrides (apply across all plates).
    pub user_overrides: std::collections::HashMap<String, String>,
    /// File-level 3MF metadata (Title, Designer, License, …)
    /// preserved across save/load.
    pub file_metadata: std::collections::BTreeMap<String, String>,
    /// Scene-wide mesh registry. Headers only; the frontend
    /// follows up per-mesh with `scene_mesh_buffers(id)` for
    /// the binary vertex / normal / index data.
    pub meshes: Vec<MeshHeader>,
    /// All plates in declaration order. Renderer routes per-plate
    /// events to the matching entry by `plate_id`.
    pub plates: Vec<PlateSnapshot>,
    /// Stable id of the currently-active plate. Frontend renders
    /// the matching `PlateSnapshot` as the foreground workspace
    /// while presenting the others as tab affordances.
    pub active_plate_id: crate::core::project::PlateId,
}

/// Per-plate slice of the snapshot — everything one plate's UI
/// surface needs to render: identity / name / printer / metadata /
/// bindings + scene contents (objects, selection, bed, exclusion
/// zones, project + per-object overrides).
#[derive(Debug, Clone, Serialize)]
pub struct PlateSnapshot {
    // ---- Plate identity / metadata ----------------------------
    pub plate_id: crate::core::project::PlateId,
    pub name: String,
    pub metadata: crate::core::project::PlateMetadata,
    /// Vendor printer identity derived from the bound
    /// `PrinterInstance.vendor_profile_ref`. Snapshot-only field
    /// for the frontend's chip + cascade context — the in-memory
    /// `Plate` only carries `printer_instance_id`. `None` for
    /// unbound plates or when the bound id no longer resolves.
    pub printer_identity: Option<String>,
    /// Plate's bound `PrinterInstance` id. The sole carrier of
    /// binding state on the plate; drives the slicer composer +
    /// the slot-binding panel.
    pub printer_instance_id: Option<String>,
    /// Plate's material → slot routing. The slot-binding panel
    /// reads this to render the per-material slot picker; auto-bind
    /// populates it on object register.
    pub material_to_slot: std::collections::BTreeMap<u8, crate::core::printer::SlotRef>,
    pub project_overrides: std::collections::HashMap<String, String>,

    // ---- Per-plate scene contents -----------------------------
    pub objects: Vec<SceneObject>,
    pub selection: Vec<ObjectId>,
    /// Active build plate identity + transform on this plate
    /// (the bed surface selection — distinct from the
    /// multi-plate `plate_id` field above).
    pub build_plate: Option<ActivePlate>,
    pub exclusion_zones: Vec<ExclusionZone>,
    pub bed: Option<BedMesh>,
    pub object_overrides:
        std::collections::HashMap<ObjectId, std::collections::HashMap<String, String>>,
}

/// Snapshot of the scene state. Frontend calls this on startup /
/// reconnect to rebuild its local mirror from scratch. Heavy mesh
/// buffers are *not* included here — the renderer follows up with
/// `scene_mesh_buffers(mesh_id)` per mesh to fetch the binary data.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn scene_snapshot(state: State<Arc<Mutex<Project>>>) -> Result<SceneSnapshot, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let meshes = s.meshes.values().map(|m| m.header()).collect();
    let plates: Vec<PlateSnapshot> = s.plates.iter().map(plate_snapshot).collect();
    let active_plate_id = s.active_plate().id;
    let _ = HashSet::<ObjectId>::new(); // silence unused-import on certain feature builds
    Ok(SceneSnapshot {
        project_uuid: s.uuid.to_string(),
        source_path: s
            .source_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        user_overrides: s.user_overrides.clone(),
        file_metadata: s.file_metadata.clone(),
        meshes,
        plates,
        active_plate_id,
    })
}

fn plate_snapshot(plate: &crate::core::project::Plate) -> PlateSnapshot {
    let mut selection: Vec<ObjectId> = plate.scene.selection.iter().copied().collect();
    selection.sort();
    let printer_identity = plate
        .printer_instance_id
        .as_deref()
        .and_then(crate::core::printer::lookup_instance)
        .map(|inst| inst.vendor_profile_ref);
    PlateSnapshot {
        plate_id: plate.id,
        name: plate.name.clone(),
        metadata: plate.metadata.clone(),
        printer_identity,
        printer_instance_id: plate.printer_instance_id.clone(),
        material_to_slot: plate.material_to_slot.clone(),
        project_overrides: plate.project_overrides.clone(),
        objects: plate.scene.objects.values().cloned().collect(),
        selection,
        build_plate: plate.scene.plate.clone(),
        exclusion_zones: plate.scene.exclusion_zones.clone(),
        bed: plate.scene.bed.clone(),
        object_overrides: plate.scene.object_overrides.clone(),
    }
}

/// Install the active printer's bed visualization + bounds. Pass
/// `None` to clear (project closed / no printer selected).
#[tauri::command]
#[tracing::instrument(skip(state, window, printer))]
pub fn scene_set_active_printer(
    printer: Option<PrinterProfile>,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_active_printer(printer.as_ref());
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Install the bundled Bambu A1 mini profile as the active printer.
/// Pulled from the printer catalog (`core::printer::registry`) so
/// the profile — including its bed-derived `supported_build_plates`
/// — comes from a single source of truth.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_default_printer(
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<PrinterProfile, String> {
    let printer = crate::core::printer::registry::lookup("bambu-lab-a1-mini")
        .ok_or_else(|| "bundled `bambu-lab-a1-mini` profile missing from catalog".to_owned())?;
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_active_printer(Some(&printer));
    drop(s);
    emit_all(&window, &events);
    Ok(printer)
}

/// Append a new plate. Active plate is unchanged. `printerIdentity`
/// is optional — new plates may be created unbound and assigned a
/// printer later via [`scene_rebind_plate_printer`]. When supplied,
/// the backend resolves the identity to a `PrinterInstance` from
/// the bundled library.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_add_plate(
    printer_identity: Option<String>,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<PlateId, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (id, events) = s.add_plate(printer_identity);
    drop(s);
    emit_all(&window, &events);
    Ok(id)
}

/// Remove the plate with the given id. Errors if it would leave
/// the project empty or if the id is unknown.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_remove_plate(
    plate_id: PlateId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.remove_plate(plate_id).map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Switch the active plate. No-op (no event) when already active.
/// Errors if the id is unknown.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_set_active_plate(
    plate_id: PlateId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.set_active_plate(plate_id).map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Rename a plate (backs the tab strip dblclick-rename).
/// Trims whitespace; rejects empty / over-`PLATE_NAME_MAX` results.
/// No-op (no event) when the trimmed value matches the current name.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_rename_plate(
    plate_id: PlateId,
    name: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .set_plate_name(plate_id, name)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Upsert one per-object cascade override on a specific plate
/// (replaces stub `onSetObjectOverride`).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_override_set(
    plate_id: PlateId,
    object_id: ObjectId,
    key: String,
    value: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .object_override_set(plate_id, object_id, key, value)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Drop one per-object cascade override key. Silent no-op when
/// the override wasn't present.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_override_clear(
    plate_id: PlateId,
    object_id: ObjectId,
    key: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .object_override_clear(plate_id, object_id, &key)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Install a printer on the specified plate. Pass `None` to clear
/// that plate's bed. The cascade re-resolution triggers naturally
/// from the BedChanged event flow.
#[tauri::command]
#[tracing::instrument(skip(state, window, printer))]
pub fn scene_set_plate_printer(
    plate_id: PlateId,
    printer: Option<PrinterProfile>,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .set_plate_printer(plate_id, printer.as_ref())
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Bundled printer-profile catalog. Returns the
/// display-ready summary the picker UI renders. Static data —
/// no project state needed.
#[tauri::command]
#[tracing::instrument]
pub fn printer_catalog() -> Vec<crate::core::printer::CatalogEntry> {
    crate::core::printer::bundled_catalog()
}

/// Rebind a plate to a `PrinterInstance` the picker chose. The
/// Tauri layer resolves the instance + its bound printer profile
/// via the registry; the mutation handles the binding update, bed
/// recompute, and report. The bed lives on the bound instance —
/// change it via `printer_instance_set_bed`.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_rebind_plate_printer(
    plate_id: PlateId,
    instance_id: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<crate::core::scene::events::PrinterChangeReport, String> {
    let instance = crate::core::printer::lookup_instance(&instance_id)
        .ok_or_else(|| format!("no printer instance with id `{instance_id}`"))?;
    let profile = crate::core::printer::lookup(&instance.vendor_profile_ref).ok_or_else(|| {
        format!(
            "printer instance `{instance_id}` references unknown vendor profile `{}`",
            instance.vendor_profile_ref,
        )
    })?;
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (report, events) = s
        .rebind_plate_printer(plate_id, instance_id, &profile)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(report)
}

/// Clear a plate's printer binding. Companion to
/// `scene_rebind_plate_printer` for the case where there's no
/// fallback printer to rebind to — e.g. the user deleted the last
/// registered instance and the workspace is about to transition
/// to the add-printer empty state.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_unbind_plate_printer(
    plate_id: PlateId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .unbind_plate_printer(plate_id)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Move an object from one plate to another. Returns
/// a `MoveReport` describing whether the world-space position
/// had to be reset (out-of-bounds, on-exclusion-zone, or
/// below-bed on the target plate).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_move_object(
    from_plate: PlateId,
    to_plate: PlateId,
    object_id: ObjectId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<MoveReport, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (report, events) = s
        .move_object(from_plate, to_plate, object_id)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(report)
}

/// Wipe every cascade override on an object.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_override_clear_all(
    plate_id: PlateId,
    object_id: ObjectId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .object_override_clear_all(plate_id, object_id)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Upsert one project-tier cascade override on a plate. Mirrors
/// `scene_object_override_set` one tier up. Silent backend no-op
/// when the value is unchanged.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_project_override_set(
    plate_id: PlateId,
    key: String,
    value: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project_override_set(plate_id, key, value)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Drop one project-tier override key from a plate. Silent no-op
/// when the key wasn't present.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_project_override_clear(
    plate_id: PlateId,
    key: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project_override_clear(plate_id, &key)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Wipe every project-tier override on a plate.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_project_override_clear_all(
    plate_id: PlateId,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project_override_clear_all(plate_id)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Upsert one user-tier (project-wide) override. The project-level
/// plugin surface writes `plugin.<name>.*` keys here; project-wide so no
/// plate id. Silent no-op when unchanged.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_user_override_set(
    key: String,
    value: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.user_override_set(key, value).map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Drop one user-tier override key. Silent no-op when absent.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_user_override_clear(
    key: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.user_override_clear(&key).map_err(|e| e.to_string())?;
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
#[tracing::instrument(skip(calibration_root))]
pub fn library_calibration(
    printer_model: String,
    calibration_root: String,
) -> Vec<super::library::CalibrationDescriptor> {
    super::library::list_calibration(&printer_model, std::path::Path::new(&calibration_root))
}

#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn library_imported(
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
) -> Result<Vec<ObjectId>, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let Some(bed) = s.active_plate().scene.bed.clone() else {
        return Ok(vec![]);
    };
    let plate_id = s.active_plate().id;
    let plan = super::arrange::plan_arrangement(&s, &bed);
    let (mut events, un_placed) = super::arrange::apply_arrangement(&mut s, plan);
    drop(s);
    if !un_placed.is_empty() {
        events.push(SceneEvent::AutoArrangeOverflow {
            plate_id,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
pub fn scene_deselect(window: Window, state: State<Arc<Mutex<Project>>>) -> Result<(), String> {
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
) -> Result<ObjectId, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let (new_id, events) = s.duplicate_object(id).map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(new_id)
}

fn op_err_to_string(e: SceneOpError) -> String {
    e.to_string()
}

/// Load a mesh from a file path (STL or OBJ) and register it as a
/// scene object at origin. The path-based form is the only public
/// load surface today; procedural-primitive registration will
/// eventually reach the same code path via a `library_*` command
/// set.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_mesh_from_path(
    path: String,
    window: Window,
    state: State<Arc<Mutex<Project>>>,
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
    state: State<Arc<Mutex<Project>>>,
) -> Result<LoadedProject, String> {
    use super::state::NewSceneObject;
    use crate::core::threemf;

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
        all_events.push(SceneEvent::MeshLoaded { mesh: header });
        mesh_ids.push(mesh_id);
    }

    let active_plate_id = s.active_plate().id;
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
            group_id: obj.group_id,
        });
        let obj_clone = s
            .active_plate()
            .scene
            .objects
            .get(&object_id)
            .unwrap()
            .clone();
        all_events.push(SceneEvent::ObjectAdded {
            plate_id: active_plate_id,
            object: obj_clone,
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::project::{PlateId, Project};
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;

    fn unit_cube_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    #[test]
    fn plate_snapshot_carries_metadata_and_scene() {
        let mut p = Project::default();
        p.plates[0].printer_instance_id = Some("bambi".into());
        p.plates[0].name = "My Plate".into();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let obj_id = p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: "cube".into(),
            visible: true,
            extruder_id: Some(2),
            parent: None,
            group_id: None,
        });

        let snap = plate_snapshot(&p.plates[0]);
        assert_eq!(snap.plate_id, PlateId(1));
        assert_eq!(snap.name, "My Plate");
        // printer_identity is derived from the bound instance.
        assert_eq!(snap.printer_identity.as_deref(), Some("bambu-lab-a1-mini"));
        assert_eq!(snap.objects.len(), 1);
        assert_eq!(snap.objects[0].id, obj_id);
        assert_eq!(snap.objects[0].extruder_id, Some(2));
    }

    #[test]
    fn plate_snapshot_selection_is_sorted() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let a = p.register_object(NewSceneObject::at_origin(mesh_id, "a"));
        let b = p.register_object(NewSceneObject::at_origin(mesh_id, "b"));
        let c = p.register_object(NewSceneObject::at_origin(mesh_id, "c"));
        // Insert in non-sorted order.
        p.plates[0].scene.selection.insert(b);
        p.plates[0].scene.selection.insert(a);
        p.plates[0].scene.selection.insert(c);

        let snap = plate_snapshot(&p.plates[0]);
        assert_eq!(snap.selection, vec![a, b, c]);
    }

    #[test]
    fn plate_snapshot_serializes_with_plate_id() {
        let p = Project::default();
        let snap = plate_snapshot(&p.plates[0]);
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["plate_id"], 1);
        assert_eq!(json["name"], "Plate 1");
    }
}
