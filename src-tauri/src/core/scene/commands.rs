//! Tauri commands that drive the scene side of [`Project`].
//!
//! Each command takes a `Window` + `State<Arc<Mutex<Session>>>`, locks
//! the state, calls a pure `Project` mutation method, emits the
//! returned events via `Window::emit`, and returns the result. Tests
//! for the *behavior* live in `core::project::mutation` against the
//! pure methods; this file only validates the Tauri plumbing.

use super::bed::BedMesh;
use super::events::{SceneEvent, SceneOpError, SelectMode};
use super::state::{
    ExclusionZone, Group, GroupId, HoleMarker, MeshHeader, MeshId, ModifierKind, ObjectId,
    SceneObject,
};
use super::transform::Transform;
use crate::core::project::{PlateId, PlateRuntime, Session};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
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
    crate::core::project::dirty::track(window, events);
    crate::core::project::history::track(window, events);
}

/// JSON-friendly snapshot of the entire project. The frontend
/// calls `scene_snapshot` on startup / reconnect to rebuild its
/// local mirror from scratch; subsequent updates arrive as scoped
/// `SceneEvent`s (each carrying its `plate_id`).
///
/// Mesh buffers are *not* included — only their headers. Geometry stays
/// Rust-side; the wgpu renderer uploads it straight to the GPU.
#[derive(Debug, Clone, Serialize)]
pub struct SceneSnapshot {
    /// Stable per-project identifier baked at project creation.
    pub project_uuid: String,
    /// Filesystem path the project was loaded from (or `None`
    /// for an unsaved in-memory project).
    pub source_path: Option<String>,
    /// For a crash-recovered project (`source_path` is `None`): the path
    /// it was loaded from before the crash. The file menu uses it as the
    /// Save-As default so Save writes back over the original.
    pub recovery_origin: Option<String>,
    /// User-tier cascade overrides (apply across all plates).
    pub user_overrides: std::collections::HashMap<String, String>,
    /// File-level 3MF metadata (Title, Designer, License, …)
    /// preserved across save/load.
    pub file_metadata: std::collections::BTreeMap<String, String>,
    /// Scene-wide mesh registry. Headers only; the binary vertex /
    /// normal / index data stays Rust-side and is uploaded straight
    /// to the GPU, never crossing IPC.
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
    /// The plate's process/quality profile override (a bundled
    /// process-fragment slug), or `None` to inherit the bound instance's
    /// `quality_profile`. Drives the per-plate Quality picker.
    pub quality_profile: Option<String>,

    // ---- Per-plate scene contents -----------------------------
    pub objects: Vec<SceneObject>,
    pub selection: Vec<ObjectId>,
    pub exclusion_zones: Vec<ExclusionZone>,
    pub bed: Option<BedMesh>,
    pub object_overrides:
        std::collections::HashMap<ObjectId, std::collections::HashMap<String, String>>,
    /// Per-group state keyed by [`GroupId`] — display name + the group's
    /// cascade overrides. A group is a set of objects sharing a
    /// `SceneObject::group`.
    pub groups: std::collections::HashMap<GroupId, Group>,
}

/// Snapshot of the scene state. Frontend calls this on startup /
/// reconnect to rebuild its local mirror from scratch. Heavy mesh
/// buffers are *not* included here — geometry stays Rust-side and
/// the wgpu renderer uploads it straight to the GPU.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn scene_snapshot(state: State<Arc<Mutex<Session>>>) -> Result<SceneSnapshot, String> {
    let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let meshes = s.project.meshes.values().map(|m| m.header()).collect();
    let plates: Vec<PlateSnapshot> = s
        .project
        .plates
        .iter()
        .map(|p| plate_snapshot(p, s.plate_runtime(p.id)))
        .collect();
    let active_plate_id = s.project.active_plate().id;
    let _ = HashSet::<ObjectId>::new(); // silence unused-import on certain feature builds
    Ok(SceneSnapshot {
        project_uuid: s.project.uuid.to_string(),
        source_path: s
            .runtime
            .source_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        recovery_origin: s
            .runtime
            .recovery_origin
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        user_overrides: s.project.user_overrides.clone(),
        file_metadata: s.project.file_metadata.clone(),
        meshes,
        plates,
        active_plate_id,
    })
}

fn plate_snapshot(
    plate: &crate::core::project::Plate,
    runtime: Option<&PlateRuntime>,
) -> PlateSnapshot {
    let mut selection: Vec<ObjectId> = runtime
        .map(|rt| rt.selection.iter().copied().collect())
        .unwrap_or_default();
    selection.sort();
    // Exclusion zones are the bed's — the scene no longer mirrors them.
    let bed = runtime.and_then(|rt| rt.bed.clone());
    let exclusion_zones = bed
        .as_ref()
        .map(|b| b.exclusion_zones.clone())
        .unwrap_or_default();
    let printer_identity = crate::core::project::session::resolve_plate_instance(plate)
        .map(|inst| inst.vendor_profile_ref);
    PlateSnapshot {
        plate_id: plate.id,
        name: plate.name.clone(),
        printer_identity,
        printer_instance_id: plate.printer_instance_id().map(str::to_owned),
        material_to_slot: plate.material_to_slot.clone(),
        project_overrides: plate.project_overrides.clone(),
        quality_profile: plate.quality_profile.clone(),
        objects: plate.scene.objects.values().cloned().collect(),
        selection,
        exclusion_zones,
        bed,
        object_overrides: plate.scene.object_overrides.clone(),
        groups: plate.scene.groups.clone(),
    }
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<PlateId, String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    // Precedence: caller's choice → the active plate's printer (add_plate
    // inherits it) → the user's last-selected printer → unbound. We only
    // inject the last-selected when the active plate is unbound (otherwise
    // add_plate's own inheritance handles it).
    let printer_identity = printer_identity.or_else(|| {
        if s.project.active_plate().printer_instance_id().is_some() {
            return None;
        }
        let pref = crate::core::config::load().defaults.printer_instance?;
        // Only use it if it's still a registered instance.
        crate::core::printer::lookup_instance(&pref).map(|_| pref)
    });
    let (id, events) = s.project.add_plate(printer_identity);
    s.reconcile(); // new plate → derive its bed + selection entry
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .remove_plate(plate_id)
        .map_err(|e| e.to_string())?;
    s.reconcile(); // drop the removed plate's runtime entry
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .set_active_plate(plate_id)
        .map_err(|e| e.to_string())?;
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    // Backend backstop for the frontend's Object-tab gate: only object/
    // region-scoped settings are honored per object — the slice path drops
    // the rest. Reject at the IPC boundary so an inert override can never be
    // persisted into the project, rather than leaning on the UI alone.
    if !crate::core::schema::is_object_overridable(&key) {
        return Err(format!(
            "`{key}` is not an object-scoped setting; it can't be set as a per-object override",
        ));
    }
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .object_override_clear(plate_id, object_id, &key)
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
    state: State<Arc<Mutex<Session>>>,
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
    let previous = s.plate_instance(plate_id);
    let (report, events) = s
        .project
        .rebind_plate_printer(
            plate_id,
            instance_id.clone(),
            &profile,
            previous.as_ref(),
            Some(&instance),
        )
        .map_err(|e| e.to_string())?;
    s.reconcile(); // rebind → re-derive the plate's bed
    drop(s);
    // Remember the user's selection as the default for new plates + projects.
    // Best-effort — a config write failure must not fail the rebind.
    if let Err(e) = crate::core::config::set_default_printer_instance(&instance_id) {
        tracing::warn!(error = %e, "failed to persist default printer instance");
    }
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .unbind_plate_printer(plate_id)
        .map_err(|e| e.to_string())?;
    s.reconcile(); // unbind → clear the plate's bed
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Move a set of objects from one plate to another, preserving their
/// world transforms (the "Send to plate" action — keeps each object's
/// authored XYZ, unlike auto-arrange). Whole groups move together and
/// the moved materials' slot bindings follow. No-op for an empty set.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_move_objects_to_plate(
    from_plate: PlateId,
    to_plate: PlateId,
    object_ids: Vec<ObjectId>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let target_instance = s.plate_instance(to_plate);
    let events = s
        .move_objects_to_plate(from_plate, to_plate, &object_ids, target_instance.as_ref())
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .project_override_clear(plate_id, &key)
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .user_override_set(key, value)
        .map_err(|e| e.to_string())?;
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .user_override_clear(&key)
        .map_err(|e| e.to_string())?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Add a procedural primitive to the scene at plate origin. Mesh
/// data is deduplicated within the scene — re-clicking "Add cube"
/// with the same parameters reuses the existing MeshId.
/// Greedy auto-arrange the current scene's visible objects onto
/// the active plate. No-op (returns empty placed/un_placed) when
/// no printer is active. `spacing_mm` (clamped to 0..=100 so a typo
/// can't wedge the nester) and `allow_rotations` come straight from
/// the arrange tool panel — UI state, not persisted anywhere.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_auto_arrange(
    spacing_mm: f32,
    allow_rotations: bool,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<Vec<ObjectId>, String> {
    if !spacing_mm.is_finite() {
        return Err("spacing must be a number".into());
    }
    let opts = super::arrange::ArrangeOptions {
        spacing_mm: spacing_mm.clamp(0.0, 100.0),
        allow_rotations,
    };
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let Some(bed) = s.active_plate_runtime().bed.clone() else {
        return Ok(vec![]);
    };
    let plate_id = s.project.active_plate().id;
    let plan =
        super::arrange::plan_arrangement(&s.project, &bed, opts, s.active_plate_instance().as_ref());
    let (mut events, un_placed) = super::arrange::apply_arrangement(&mut s, plan);
    s.reconcile(); // spill may have added plates → derive their beds
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

/// Clone the given objects on the active plate. Per-object settings and group
/// structure are duplicated with the geometry (see [`Project::clone_objects`]).
///
/// - `copies = Some(n)`: make exactly `n` copies of the whole `ids` set,
///   stacked in place on their originals (no auto-arrange — the user positions
///   them).
/// - `copies = None`: "fill plate" — clone the set one copy at a time, packing
///   the plate with the nester after each, until the next copy would spill onto
///   another plate, then stop. Needs an active printer (its bed bounds drive the
///   fit test); errors without one.
///
/// `expand_groups` (like the orient/align tools): when set, expand `ids` to
/// whole groups first, so cloning a single picked volume clones its siblings.
///
/// Returns the new object ids.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_clone(
    ids: Vec<ObjectId>,
    copies: Option<u32>,
    expand_groups: Option<bool>,
    // Fill-plate nester options from the clone panel (same knobs as
    // auto-arrange). Ignored for `copies = Some(n)` (no packing there);
    // absent → defaults.
    spacing_mm: Option<f32>,
    allow_rotations: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<Vec<ObjectId>, String> {
    if ids.is_empty() {
        return Err("clone: no objects selected".into());
    }
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let session = &mut *s;
    // Order the selection through the active plate's object list — `clone_objects`
    // requires an `OrderedIds`, so the clones can't inherit the wire order.
    let ids = if expand_groups.unwrap_or(false) {
        session.project.group_expanded_ids(&ids)
    } else {
        session.project.active_plate().scene.objects.in_order(&ids)
    };
    let bed = session.active_plate_runtime().bed.clone();
    let instance = session.active_plate_instance();
    let mut events = Vec::new();
    let new_ids;

    match copies {
        Some(n) => {
            // Clone in place — no arrange; the copies stack on their originals
            // and the user moves them where they want.
            let (ids_new, evs) = session.project.clone_objects(&ids, n, instance.as_ref());
            new_ids = ids_new;
            events.extend(evs);
        }
        None => {
            let Some(bed) = bed.as_ref() else {
                return Err("Fill plate needs an active printer.".into());
            };
            // Clone one copy at a time, keeping the last packing that still fits
            // this single plate. ponytail: hard cap so a tiny part on a huge bed
            // can't loop unbounded — 1000 copies is well past useful; warn if hit.
            const MAX_COPIES: u32 = 1000;
            let defaults = super::arrange::ArrangeOptions::default();
            let opts = super::arrange::ArrangeOptions {
                spacing_mm: spacing_mm
                    .filter(|v| v.is_finite())
                    .map(|v| v.clamp(0.0, 100.0))
                    .unwrap_or(defaults.spacing_mm),
                allow_rotations: allow_rotations.unwrap_or(defaults.allow_rotations),
            };
            let mut kept_ids = Vec::new();
            let mut last_plan = None;
            let mut hit_cap = true;
            for _ in 0..MAX_COPIES {
                let (batch, evs) = session.project.clone_objects(&ids, 1, instance.as_ref());
                let plan = super::arrange::plan_arrangement(
                    &session.project,
                    bed,
                    opts,
                    session.active_plate_instance().as_ref(),
                );
                if plan.spilled.is_empty() && plan.un_placed.is_empty() {
                    events.extend(evs);
                    kept_ids.extend(batch);
                    last_plan = Some(plan);
                } else {
                    // This copy overflows the plate — undo it and stop. Its
                    // ObjectAdded events were never emitted, so the matching
                    // ObjectRemoved ones are dropped too: the copy never existed
                    // for the UI.
                    let _ = session.delete_objects(&batch);
                    hit_cap = false;
                    break;
                }
            }
            if hit_cap {
                tracing::warn!(max = MAX_COPIES, "fill-plate hit the copy cap");
            }
            // Apply the last packing that fit (repositions originals + all kept
            // copies on the single plate).
            if let Some(plan) = last_plan {
                let (evs, _) = super::arrange::apply_arrangement(session, plan);
                events.extend(evs);
            }
            new_ids = kept_ids;
        }
    }

    drop(s);
    emit_all(&window, &events);
    Ok(new_ids)
}

#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_add_from_primitive(
    kind: super::primitives::PrimitiveKind,
    // Optional — omitted by the object-library quick-add, which wants the
    // kind's sensible defaults. A future parameter dialog supplies them.
    params: Option<super::primitives::PrimitiveParams>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(MeshId, ObjectId), String> {
    let params = params.unwrap_or_else(|| super::primitives::PrimitiveParams::defaults_for(kind));
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let instance = s.active_plate_instance();
    let (mesh_id, obj_id, events) = s.add_from_primitive(kind, params, instance.as_ref());
    drop(s);
    emit_all(&window, &events);
    Ok((mesh_id, obj_id))
}

/// Replace / add / toggle the selection.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_select(
    ids: Vec<ObjectId>,
    mode: SelectMode,
    // When set, expand the ids to whole groups before selecting (the
    // canvas's click-selects-the-group behaviour). The object list omits
    // it so parts stay individually selectable.
    expand_groups: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let ids = if expand_groups.unwrap_or(false) {
        s.project.group_expanded_ids(&ids).to_vec()
    } else {
        ids
    };
    let events = s.select(&ids, mode);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Clear the selection.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_deselect(window: Window, state: State<Arc<Mutex<Session>>>) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.deselect_all();
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.delete_objects(&ids);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Engine "Auto orient": run libslic3r's support-minimizing orientation
/// optimizer on the current selection and apply the result. The selection is
/// oriented as one rigid unit (combined world mesh → one rotation → rotate all
/// about the shared center), so a group/assembly keeps its arrangement. The
/// optimizer can run for a noticeable time, so the combined mesh is read out and
/// the scene lock released while it runs, then re-acquired to apply the result.
///
/// `expand_groups` supports the selection-less pick path: with nothing
/// selected, the click identifies a single object, and we expand it to its
/// whole group so the group is oriented as one unit. Expanded once up front so
/// the read-out mesh and the applied rotation cover the same id set.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_auto_orient(
    mut ids: Vec<ObjectId>,
    expand_groups: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let (vertices, indices) = {
        let s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        if expand_groups.unwrap_or(false) {
            ids = s.project.group_expanded_ids(&ids).to_vec();
        }
        s.project
            .objects_world_mesh(&ids)
            .map_err(op_err_to_string)?
    };
    let quat = slic3r_ffi::orient_mesh(&vertices, &indices, None)
        .map_err(|e| format!("auto-orient failed: {e}"))?;
    let rotation = glam::Quat::from_xyzw(quat[0], quat[1], quat[2], quat[3]);
    let events = {
        let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        s.project
            .orient_objects(&ids, rotation, None)
            .map_err(op_err_to_string)?
    };
    emit_all(&window, &events);
    Ok(())
}

/// Split tool: cut the selection by a world-space plane and keep/discard each
/// side. Mirrors auto-orient's three phases (read targets under lock → cut via
/// the FFI unlocked → register under lock). The selection is group-expanded so
/// a whole group is cut; each object is cut in its own local frame (the world
/// plane is transformed into local coords), so the halves reuse the source
/// transform. Kept, non-empty halves become new objects; sources are removed.
/// Returns the new object ids (selected). `keep_positive`/`negative` choose the
/// side(s) the plane normal points toward / away from.
/// One reassembly connector, in WORLD plane-space: `(u, v)` in the plane's
/// in-plane basis, params as lowercase strings (`type`: plug/dowel/snap,
/// `style`: prism/frustum, `shape`: triangle/square/hexagon/circle).
#[derive(serde::Deserialize, Clone)]
pub struct CutConnectorInput {
    pub u: f32,
    pub v: f32,
    #[serde(default)]
    pub z_angle: f32,
    pub radius: f32,
    pub height: f32,
    #[serde(default)]
    pub r_tol: f32,
    #[serde(default)]
    pub h_tol: f32,
    #[serde(rename = "type")]
    pub ty: String,
    pub style: String,
    pub shape: String,
}

fn map_connector(c: &CutConnectorInput, pos: [f32; 3]) -> slic3r_ffi::Connector {
    use slic3r_ffi::{ConnectorShape, ConnectorStyle, ConnectorType};
    let ty = match c.ty.as_str() {
        "dowel" => ConnectorType::Dowel,
        "snap" => ConnectorType::Snap,
        _ => ConnectorType::Plug,
    };
    let style = match c.style.as_str() {
        "frustum" => ConnectorStyle::Frustum,
        _ => ConnectorStyle::Prism,
    };
    let shape = match c.shape.as_str() {
        "triangle" => ConnectorShape::Triangle,
        "square" => ConnectorShape::Square,
        "hexagon" => ConnectorShape::Hexagon,
        _ => ConnectorShape::Circle,
    };
    slic3r_ffi::Connector {
        pos,
        radius: c.radius,
        height: c.height,
        r_tol: c.r_tol,
        h_tol: c.h_tol,
        z_angle: c.z_angle,
        ty,
        style,
        shape,
    }
}

#[tauri::command]
#[tracing::instrument(skip(state, window, connectors))]
pub async fn scene_cut_apply(
    ids: Vec<ObjectId>,
    plane_origin: [f32; 3],
    plane_normal: [f32; 3],
    keep_positive: bool,
    keep_negative: bool,
    connectors: Vec<CutConnectorInput>,
    window: Window,
    state: State<'_, Arc<Mutex<Session>>>,
) -> Result<Vec<u64>, String> {
    use crate::core::project::mutation::{CutHalfOut, CutResult, CutSide};
    if !keep_positive && !keep_negative {
        return Err("split: at least one side must be kept".into());
    }
    let normal = glam::Vec3::from(plane_normal);
    if normal.length() < 1e-6 {
        return Err("split: degenerate plane normal".into());
    }
    let origin = glam::Vec3::from(plane_origin);
    // In-plane basis (mirrors the renderer's up-pick) → each connector's WORLD
    // position from its plane-space (u, v). Dowel pins only make sense when both
    // halves are kept; otherwise drop them.
    let up = if normal.x.abs() > 0.9 {
        glam::Vec3::Y
    } else {
        glam::Vec3::X
    };
    let e1 = normal.cross(up).normalize();
    let e2 = normal.cross(e1).normalize();
    let conn_world: Vec<glam::Vec3> = connectors
        .iter()
        .map(|c| origin + e1 * c.u + e2 * c.v)
        .collect();
    let keep_dowels = keep_positive && keep_negative;

    // The cut (FFI mesh booleans) runs on the blocking pool, not the main/UI
    // thread — a large mesh would otherwise freeze the window for its duration.
    let project = Arc::clone(state.inner());
    let results: Vec<CutResult> = tauri::async_runtime::spawn_blocking(move || {
        // Phase 1: read the group-expanded targets under the lock, then release.
        let targets = {
            let s = project.lock().map_err(|e| format!("scene lock: {e}"))?;
            let ids = s.project.group_expanded_ids(&ids).to_vec();
            s.project.cut_targets(&ids).map_err(op_err_to_string)?
        };
        // Phase 2: cut each target in its own local frame (FFI, unlocked). A
        // source is recorded even when fully discarded (no kept half) so it gets
        // removed.
        let mut results: Vec<CutResult> = Vec::new();
        for t in &targets {
            let m = t.transform.to_mat4();
            let inv = m.inverse();
            let o_l = inv.transform_point3(origin);
            // World→local normal uses the transpose of the linear part.
            let n_l = (glam::Mat3::from_mat4(m).transpose() * normal).normalize();
            // Each connector's world point → this object's local frame.
            let conns: Vec<slic3r_ffi::Connector> = connectors
                .iter()
                .zip(&conn_world)
                .map(|(c, w)| map_connector(c, inv.transform_point3(*w).to_array()))
                .collect();
            // Deferred cut: connectors come back as separate volumes (peg/hole)
            // instead of baked booleans — the slice path subtracts holes in 2D.
            let res = slic3r_ffi::cut_mesh_deferred(
                &t.vertices,
                &t.indices,
                o_l.to_array(),
                n_l.to_array(),
                &conns,
                t.paint.as_deref().map(Vec::as_slice),
            )
            .map_err(|e| format!("split: cut failed: {e}"))?;
            // Route each connector volume to its half (0 = pos, 1 = neg).
            let (mut pos_mods, mut neg_mods) = (Vec::new(), Vec::new());
            for mv in res.modifiers {
                let kind = if mv.negative {
                    ModifierKind::Hole
                } else {
                    ModifierKind::Peg
                };
                let entry = (mv.vertices, mv.indices, kind);
                if mv.half == 0 {
                    pos_mods.push(entry);
                } else {
                    neg_mods.push(entry);
                }
            }
            // Display-only marker: where each hole opens on the cut face (the
            // hole volume isn't baked into the half mesh). Every connector opens
            // a hole on the pos face (plug/snap hole; dowel hole); a dowel opens
            // one on the neg face too. The viewport shades these onto the cap.
            // Vertex-0 direction of the connector cross-section, object-local, so
            // the decal's corners align with the real peg/hole. Mirrors the FFI:
            // align the normal to +Z (shortest arc), place vertex 0 at +Y, spin by
            // -z_angle, rotate back. (shortest-arc rotation is unique, so glam
            // reproduces the shim's `FromTwoVectors` exactly for non-antiparallel
            // normals.)
            let q = glam::Quat::from_rotation_arc(n_l, glam::Vec3::Z);
            let (mut pos_markers, mut neg_markers) = (Vec::new(), Vec::new());
            for c in &conns {
                let dir_plane = glam::Vec3::new(c.z_angle.sin(), c.z_angle.cos(), 0.0);
                let u_axis = (q.inverse() * dir_plane).normalize_or_zero();
                let marker = HoleMarker {
                    shape: c.shape.into(),
                    radius: c.radius + c.r_tol,
                    center: c.pos,
                    normal: n_l.to_array(),
                    u_axis: u_axis.to_array(),
                };
                pos_markers.push(marker);
                if matches!(c.ty, slic3r_ffi::ConnectorType::Dowel) {
                    neg_markers.push(marker);
                }
            }
            // Carry the source object's existing connectors/markers (from a prior
            // cut) onto whichever half they land on — by centroid side of the new
            // plane. A modifier straddling the plane goes to its centroid's side.
            let side_pos = |p: glam::Vec3| (p - o_l).dot(n_l) >= 0.0;
            for (verts, idx, kind) in &t.modifiers {
                let n = verts.len() / 3;
                if n == 0 {
                    continue;
                }
                let c = (0..n).fold(glam::Vec3::ZERO, |acc, k| {
                    acc + glam::Vec3::new(verts[3 * k], verts[3 * k + 1], verts[3 * k + 2])
                }) / n as f32;
                let entry = (verts.clone(), idx.clone(), *kind);
                if side_pos(c) {
                    pos_mods.push(entry);
                } else {
                    neg_mods.push(entry);
                }
            }
            for hm in &t.hole_markers {
                if side_pos(glam::Vec3::from(hm.center)) {
                    pos_markers.push(*hm);
                } else {
                    neg_markers.push(*hm);
                }
            }
            let mut halves = Vec::new();
            if keep_positive && !res.pos.is_empty() {
                halves.push(CutHalfOut {
                    side: CutSide::Pos,
                    vertices: res.pos.vertices,
                    indices: res.pos.indices,
                    paint: res.pos.paint,
                    modifiers: pos_mods,
                    hole_markers: pos_markers,
                });
            }
            if keep_negative && !res.neg.is_empty() {
                halves.push(CutHalfOut {
                    side: CutSide::Neg,
                    vertices: res.neg.vertices,
                    indices: res.neg.indices,
                    paint: res.neg.paint,
                    modifiers: neg_mods,
                    hole_markers: neg_markers,
                });
            }
            let dowels = if keep_dowels {
                res.dowels
                    .into_iter()
                    .map(|d| (d.vertices, d.indices))
                    .collect()
            } else {
                Vec::new()
            };
            results.push(CutResult {
                source_id: t.object_id,
                transform: t.transform,
                base_name: t.name.clone(),
                extruder_id: t.extruder_id,
                source_group: t.group,
                halves,
                dowels,
            });
        }
        Ok::<_, String>(results)
    })
    .await
    .map_err(|e| format!("split: cut task panicked: {e}"))??;

    if results.is_empty() {
        return Ok(Vec::new());
    }
    // Phase 3: register the halves + remove the sources, under lock.
    let (new_ids, events) = {
        let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        let instance = s.active_plate_instance();
        s.apply_cut(results, instance.as_ref())
    };
    emit_all(&window, &events);
    Ok(new_ids.into_iter().map(|id| id.0).collect())
}

/// "Align to axis": rotate the selection about Z so its dominant horizontal
/// line direction (the length-weighted most common edge direction) becomes
/// parallel to the X or Y axis. Unlike auto-orient this is a pure yaw — it
/// doesn't change which face is down, so it composes with a prior orient — and
/// needs no engine call; the angle is a cheap pure-Rust computation over the
/// selection's combined world mesh, so it all runs under one lock. A
/// near-isotropic footprint (no dominant direction) is a no-op.
///
/// `expand_groups` mirrors the other tools: the selection-less pick path passes
/// a single clicked id and expands it to its whole group.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_align_axis(
    mut ids: Vec<ObjectId>,
    axis: super::align::AlignAxis,
    expand_groups: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let events = {
        let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        if expand_groups.unwrap_or(false) {
            ids = s.project.group_expanded_ids(&ids).to_vec();
        }
        let (vertices, indices) = s
            .project
            .objects_world_mesh(&ids)
            .map_err(op_err_to_string)?;
        let Some(angle) = super::align::axis_alignment_rotation(&vertices, &indices, axis) else {
            return Ok(()); // no dominant direction — nothing to align
        };
        let rotation = glam::Quat::from_rotation_z(angle);
        s.project
            .orient_objects(&ids, rotation, None)
            .map_err(op_err_to_string)?
    };
    emit_all(&window, &events);
    Ok(())
}

/// "Align face to face": yaw the target object so its clicked face faces the
/// same way as a reference face on another object (matching the reference's
/// actual heading, not a world axis), then slide it along the reference face's
/// normal so the two clicked faces are coplanar. `ref_normal`/`ref_point` are
/// the first clicked face's world normal + hit point; `face_normal`/`face_point`
/// are the target's. The yaw depends only on the normals (computed before
/// locking); the lock expands the group and applies the yaw + coplanar slide via
/// `align_face_coplanar`, which tracks `face_point` through the in-place
/// rotation. A ~horizontal face on either side (no in-plane heading) is a no-op.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
#[allow(clippy::too_many_arguments)]
pub fn scene_object_align_face(
    mut ids: Vec<ObjectId>,
    ref_normal: [f32; 3],
    face_normal: [f32; 3],
    ref_point: [f32; 3],
    face_point: [f32; 3],
    expand_groups: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let Some(angle) = super::align::face_to_face_yaw(ref_normal, face_normal) else {
        return Ok(()); // a horizontal face on either side — nothing to match
    };
    // Slide along the reference face's in-plane normal; the coplanar target is
    // the reference point's projection onto it. `face_to_face_yaw` returning
    // `Some` guarantees the reference has a non-trivial in-plane heading.
    let n_len = (ref_normal[0] * ref_normal[0] + ref_normal[1] * ref_normal[1]).sqrt();
    let slide_dir = glam::Vec3::new(ref_normal[0] / n_len, ref_normal[1] / n_len, 0.0);
    let target_coord = ref_point[0] * slide_dir.x + ref_point[1] * slide_dir.y;
    let track = glam::Vec3::new(face_point[0], face_point[1], face_point[2]);
    let events = {
        let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        if expand_groups.unwrap_or(false) {
            ids = s.project.group_expanded_ids(&ids).to_vec();
        }
        s.project
            .align_face_coplanar(
                &ids,
                glam::Quat::from_rotation_z(angle),
                slide_dir,
                target_coord,
                track,
            )
            .map_err(op_err_to_string)?
    };
    emit_all(&window, &events);
    Ok(())
}

/// "Lay flat on…": lay a clicked face of the selection onto the plate.
/// `normal` is the clicked face's world-space outward normal (from
/// `viewport_pick_face`); the command rotates it onto -Z so the face lands flat.
/// `contact` is a world point on that face (the ray hit). The selection rotates
/// rigidly about the contact point, then drops so the contact — and its
/// now-horizontal face — sits on the plate. The clicked triangle defines the
/// contact plane, so the placement is exact (no bounding-box gap / float).
///
/// `expand_groups` supports the selection-less pick path: when the user lays
/// flat with nothing selected, the click identifies a single object, and we
/// expand it to its whole group so the group lays flat as one rigid unit. The
/// selection path leaves it unset so a panel-selected single child (a group
/// subcomponent) stays individually targetable.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_object_lay_flat_on(
    ids: Vec<ObjectId>,
    normal: [f32; 3],
    contact: [f32; 3],
    expand_groups: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let n = glam::Vec3::from(normal);
    if n.length_squared() < 1e-6 {
        return Err("lay_flat_on: degenerate normal".into());
    }
    // Shortest-arc rotation taking the clicked face's outward normal to -Z.
    let q = glam::Quat::from_rotation_arc(n.normalize(), -glam::Vec3::Z);
    let contact = glam::Vec3::new(contact[0], contact[1], contact[2]);
    let events = {
        let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
        let ids = if expand_groups.unwrap_or(false) {
            s.project.group_expanded_ids(&ids).to_vec()
        } else {
            ids
        };
        s.project
            .orient_objects(&ids, q, Some(contact))
            .map_err(op_err_to_string)?
    };
    emit_all(&window, &events);
    Ok(())
}

/// Set an object's material — its 1-based `extruder_id` — on the active
/// plate. Auto-binds the material to a slot if it had none, so the
/// material → slot table stays complete.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_set_object_material(
    id: ObjectId,
    material: u8,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let instance = s.active_plate_instance();
    let events = s
        .project
        .set_object_material(id, material, instance.as_ref())
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Group objects on the active plate into one logical (multi-volume)
/// object named `name` — the same `group` mechanism as 3MF
/// multi-volume objects.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_group_objects(
    ids: Vec<ObjectId>,
    name: String,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s
        .project
        .group_objects(&ids, name)
        .map_err(op_err_to_string)?;
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Ungroup a group on the active plate (clear its members' `group`).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_ungroup_objects(
    group: GroupId,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.project.ungroup_objects(group);
    drop(s);
    emit_all(&window, &events);
    Ok(())
}

/// Rename a group on the active plate.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_rename_group(
    group: GroupId,
    name: String,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let events = s.project.rename_group(group, name);
    drop(s);
    emit_all(&window, &events);
    Ok(())
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
    state: State<Arc<Mutex<Session>>>,
) -> Result<(MeshId, ObjectId), String> {
    let new_mesh = super::loaders::load_mesh_from_path(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;
    let instance = s.active_plate_instance();
    let (mesh_id, obj_id, events) = s.project.load_mesh(new_mesh, instance.as_ref());
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
///
/// `import_settings` (the "Add model + settings…" path) additionally
/// carries the file's settings onto the imported objects: the source
/// project's config, diffed against the active plate's effective
/// baseline and gated to object-applicable keys, applies to every
/// object; each object's own `model_settings.config` entries merge over
/// that. Filament/machine keys never come along — n3o owns the printer
/// and the filament. Without the flag the import is geometry-only
/// (objects, transforms, groups, paint, extruder hints — no settings).
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn scene_load_3mf(
    path: String,
    import_settings: Option<bool>,
    window: Window,
    state: State<Arc<Mutex<Session>>>,
) -> Result<LoadedProject, String> {
    use super::state::NewSceneObject;
    use crate::core::threemf;

    let project = threemf::load_3mf(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    let import_settings = import_settings.unwrap_or(false);

    // Parse the source project's config up front (before the state lock
    // moves `project`). A with-settings pick of a vanilla 3MF (no BBS
    // config) degrades to a geometry-only load rather than failing.
    let foreign_settings = if import_settings {
        match &project.embedded_settings {
            Some(raw) => match crate::core::orca_import::OrcaProjectSettings::parse(raw.as_bytes())
            {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("add-model-with-settings: {e}; importing geometry only");
                    None
                }
            },
            None => {
                tracing::warn!(
                    "add-model-with-settings: no project_settings.config in {path}; importing geometry only",
                );
                None
            }
        }
    } else {
        None
    };

    let mut s = state.lock().map_err(|e| format!("scene lock: {e}"))?;

    // The plate-level portion of the imported settings: the source
    // config diffed against this plate's effective baseline, restricted
    // to object-applicable keys. Computed once; applied to every
    // imported object below, under its own per-object entries.
    let plate_diff: std::collections::BTreeMap<String, String> = match &foreign_settings {
        Some(settings) => {
            let baseline = crate::core::project::resolve::plate_effective_baseline(
                &s.project,
                s.project.active_plate().id,
            )?;
            let outcome =
                crate::core::orca_import::object_applicable_overrides(settings, &baseline);
            tracing::info!(
                applied = outcome.overrides.len(),
                redundant = outcome.redundant.len(),
                incompatible = outcome.incompatible.len(),
                "add-model-with-settings: plate-level diff",
            );
            outcome.overrides
        }
        None => Default::default(),
    };

    // Register every mesh first, capturing the allocated MeshIds in
    // the same order as Project3mf.meshes so ProjectObject.mesh_idx
    // remains a valid index lookup. Collect MeshLoaded events from
    // the headers as we go.
    let mut all_events: Vec<SceneEvent> = Vec::new();
    let mut mesh_ids: Vec<MeshId> = Vec::with_capacity(project.meshes.len());
    for new_mesh in project.meshes {
        let mesh_id = s.project.register_mesh(new_mesh);
        let header = s.project.meshes.get(&mesh_id).unwrap().header();
        all_events.push(SceneEvent::MeshLoaded { mesh: header });
        mesh_ids.push(mesh_id);
    }

    let active_plate_id = s.project.active_plate().id;
    let instance = s.active_plate_instance();
    let mut loaded = Vec::with_capacity(project.objects.len());
    for obj in project.objects {
        let mesh_id = mesh_ids[obj.mesh_idx];
        let object_id = s.project.register_object(
            NewSceneObject {
                mesh: mesh_id,
                transform: obj.transform,
                name: obj.name.clone(),
                visible: true,
                extruder_id: obj.extruder_id,
                // GroupIds are globally unique, so merging a multi-part import
                // into an existing project can't collide with its groups — no
                // remap needed.
                group: obj.group,
            },
            instance.as_ref(),
        );
        // Settings ride only the "Add model + settings…" path: the
        // plate-level diff first, the object's own model_settings entries
        // over it, routed group/member exactly like panel edits. The
        // plain path imports geometry only.
        if import_settings {
            all_events.extend(s.project.apply_imported_object_settings(
                active_plate_id,
                object_id,
                &plate_diff,
                &obj.overrides,
            ));
        }
        let obj_clone = s
            .project
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
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
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
        p.plates[0].set_printer(Some("bambi".into()));
        p.plates[0].name = "My Plate".into();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let obj_id = p.register_object(
            NewSceneObject {
                mesh: mesh_id,
                transform: Transform::IDENTITY,
                name: "cube".into(),
                visible: true,
                extruder_id: Some(2),
                group: None,
            },
            None,
        );

        let snap = plate_snapshot(&p.plates[0], None);
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
        let a = p.register_object(NewSceneObject::at_origin(mesh_id, "a"), None);
        let b = p.register_object(NewSceneObject::at_origin(mesh_id, "b"), None);
        let c = p.register_object(NewSceneObject::at_origin(mesh_id, "c"), None);
        // Insert in non-sorted order into the plate's runtime selection.
        let mut rt = PlateRuntime::default();
        rt.selection.insert(b);
        rt.selection.insert(a);
        rt.selection.insert(c);

        let snap = plate_snapshot(&p.plates[0], Some(&rt));
        assert_eq!(snap.selection, vec![a, b, c]);
    }

    #[test]
    fn plate_snapshot_serializes_with_plate_id() {
        let p = Project::default();
        let snap = plate_snapshot(&p.plates[0], None);
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["plate_id"], 1);
        assert_eq!(json["name"], "Plate 1");
    }
}
