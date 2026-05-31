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

/// Map a winning rule's source path to the cascade *layer* the settings
/// panel ladder renders it under. The fragment paths `compose_cascade`
/// stamps are deterministic per step, so this is an exact classification
/// (not a heuristic): the process fragment → the `"user"` row (labeled
/// "Profile" — the selected quality/process profile), printer/bed/filament
/// fragments → their rows, synthesized machine-topology rules → printer.
/// Returns the frontend `CascadeLayer` id, or `None` for `<plate-overrides>`
/// (the panel draws override tiers itself) / anything unrecognized.
fn layer_for_source(path: &std::path::Path) -> Option<&'static str> {
    let s = path.to_string_lossy();
    if s.contains("/processes/") {
        Some("user") // the "Profile" row = quality/process profile
    } else if s.contains("/beds/") {
        Some("build_plate")
    } else if s.contains("/filament/")
        || s.contains("<filament-vector-assembly>")
        || s.contains("<filament-colour-synthesis>")
    {
        Some("filament")
    } else if s.contains("machine.toml")
        || s.contains("/nozzles/")
        || s.contains("<flush-defaults>")
        || s.contains("<extruder-vector-assembly>")
        || s.contains("<filament-topology>")
    {
        Some("printer")
    } else {
        None
    }
}

/// One resolved key for the settings-panel ladder: the cascade-resolved
/// `value` (fragments only — override tiers are drawn frontend-side) plus
/// the `source_layer` it won from.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlateResolvedEntry {
    pub value: String,
    pub source_layer: Option<String>,
}

/// The whole resolved map for a plate, keyed by libslic3r setting key.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlateResolvedJson {
    pub entries: std::collections::HashMap<String, PlateResolvedEntry>,
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
) -> Result<PlateResolvedJson, String> {
    let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    resolve_plate_cascade(&p, plate_id)
}

/// Core of [`plate_cascade_resolve`], split out so it's testable without
/// a Tauri `State`.
pub fn resolve_plate_cascade(p: &Project, plate_id: PlateId) -> Result<PlateResolvedJson, String> {
    use std::collections::{BTreeMap, HashMap};
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id.as_deref() else {
        return Ok(PlateResolvedJson {
            entries: HashMap::new(),
        });
    };
    let instance = crate::core::printer::lookup_instance(instance_id)
        .ok_or_else(|| format!("unknown printer instance `{instance_id}`"))?;
    let printer = crate::core::printer::lookup(&instance.vendor_profile_ref)
        .ok_or_else(|| format!("unknown vendor profile `{}`", instance.vendor_profile_ref))?;
    let bed_identity = instance.bed.identity.clone();
    let bed = crate::core::scene::build_plate::lookup(&bed_identity).unwrap_or_else(|| {
        crate::core::scene::build_plate::BuildPlate {
            libslic3r_curr_bed_type: format!("{bed_identity} Plate"),
            identity: bed_identity.clone(),
        }
    });
    // Filament context for `when.filament.*` predicates: one filament per
    // physical slot (always ≥1, so predicates resolve and the empty plate
    // still shows the instance's filaments). active_slot 0 → slot 0.
    //
    // Scope note: this is the instance's slot view. The slice path
    // (`slice::input`) instead fans one filament per *material* via the
    // plate's `material_to_slot`. They agree for the common case (material
    // i bound to slot i, the auto-bind default), but on an AMS printer
    // where the user has manually bound a material to a slot holding a
    // different filament *type*, a `when.filament.type`-gated value the
    // ladder shows can differ from what that material slices with. Process
    // / printer / bed rows (incl. the headline Profile attribution) are
    // unaffected. A per-material filament view here is the follow-up.
    let filaments: Vec<std::sync::Arc<crate::core::filament::FilamentProfile>> = instance
        .extruders
        .iter()
        .flat_map(|e| &e.slots)
        .map(|slot| {
            let id = slot
                .filament_identity
                .as_deref()
                .unwrap_or(instance.default_filament_fragment_slug.as_str());
            std::sync::Arc::new(crate::core::filament::lookup(id).unwrap_or_else(|| {
                crate::core::filament::FilamentProfile {
                    identity: id.to_owned(),
                    base_type: "PLA".into(),
                    vendor: None,
                    color: None,
                }
            }))
        })
        .collect();

    let effective = crate::core::profile_library::with_quality_profile(
        &instance,
        plate.quality_profile.as_deref(),
    );
    let cascade = crate::core::profile_library::compose_cascade(&effective, &[], &BTreeMap::new())
        .map_err(|e| format!("compose: {e}"))?;
    let ctx = crate::core::project::SlicingContext::new(
        std::sync::Arc::new(printer),
        std::sync::Arc::new(bed),
        filaments,
    );
    let resolved = crate::core::cascade::resolve(&cascade, &ctx);
    let entries = resolved
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                PlateResolvedEntry {
                    source_layer: layer_for_source(&v.winning_rule.path).map(str::to_owned),
                    value: v.value,
                },
            )
        })
        .collect();
    Ok(PlateResolvedJson { entries })
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
    format::write_project(&p, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    drop(p);
    emit_all(&window, &[SceneEvent::ProjectSaved { path: path.clone() }]);
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
    format::write_project(&p, std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    p.source_path = Some(PathBuf::from(&path));
    drop(p);
    emit_all(&window, &[SceneEvent::ProjectSaved { path: path.clone() }]);
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
    let path_ref = std::path::Path::new(&path);
    // Open project transparently imports a foreign OrcaSlicer / Bambu
    // Studio project (no n3o_project.json) instead of erroring.
    let (loaded, import_report) = match format::read_project(path_ref) {
        Ok(p) => (p, None),
        Err(format::ProjectIoError::ForeignProject { .. }) => {
            let (project, report) =
                crate::core::orca_import::import(path_ref).map_err(|e| format!("import: {e}"))?;
            (project, Some(report))
        }
        Err(e) => return Err(e.to_string()),
    };
    let returned = loaded.clone();
    {
        let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
        *p = loaded;
    }
    // ProjectLoaded first (scene re-syncs), then the import report.
    let mut events = vec![SceneEvent::ProjectLoaded { path: path.clone() }];
    if let Some(report) = import_report {
        events.push(SceneEvent::ProjectImported {
            path: path.clone(),
            report,
        });
    }
    emit_all(&window, &events);
    Ok(returned)
}

/// Reset the in-memory project to a fresh, empty default, **replacing**
/// the current one wholesale. Emits `project:loaded` (with an empty
/// path — there's no file yet) so the frontend mirror throws out its
/// cached scene and re-syncs via `scene_snapshot`, the same path as
/// loading a file. The new project has `source_path = None`, so the UI
/// shows it as "Untitled". The previous project's autosave file is
/// keyed by its own uuid and survives, so this stays recoverable.
#[tauri::command]
#[tracing::instrument(skip(state, window))]
pub fn project_new(window: Window, state: State<Arc<Mutex<Project>>>) -> Result<(), String> {
    {
        let mut p = state.lock().map_err(|e| format!("project lock: {e}"))?;
        *p = Project::default();
    }
    emit_all(
        &window,
        &[SceneEvent::ProjectLoaded {
            path: String::new(),
        }],
    );
    Ok(())
}

// ---- Autosave --------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn layer_for_source_maps_fragments_to_rows() {
        let p = |s: &str| layer_for_source(Path::new(s));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/processes/0.20mm-standard.toml"),
            Some("user"),
            "process fragment → Profile row",
        );
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/beds/textured-pei.toml"),
            Some("build_plate")
        );
        assert_eq!(p("generic/filament/generic-pla.toml"), Some("filament"));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/machine.toml"),
            Some("printer")
        );
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/nozzles/0.4.toml"),
            Some("printer")
        );
        assert_eq!(p("<plate-overrides>"), None);
    }

    #[test]
    fn plate_resolve_attributes_outer_wall_speed_to_the_profile_layer() {
        // FFI + the bundled profile library back compose+resolve.
        let _ = slic3r_ffi::init(None, 3);
        // Default project: plate 0 bound to the bundled A1 mini (`bambi`,
        // quality_profile = "0.20mm-standard").
        let project = Project::default();
        let plate_id = project.plates[0].id;
        // Sanity: the test env actually bound an instance.
        assert!(project.plates[0].printer_instance_id.is_some());

        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        let ow = resolved
            .entries
            .get("outer_wall_speed")
            .expect("outer_wall_speed resolved");
        // 0.20mm-standard's process fragment sets 200, attributed to the
        // "Profile" (process/quality-profile) row.
        assert_eq!(ow.value, "200");
        assert_eq!(ow.source_layer.as_deref(), Some("user"));
    }

    #[test]
    fn plate_resolve_follows_a_per_plate_quality_profile() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        // Switch this plate to the Strength preset (60), leaving the
        // instance's default (Standard, 200) untouched.
        project
            .set_plate_quality_profile(plate_id, Some("0.20mm-strength".into()))
            .expect("set strength");
        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        let ow = resolved.entries.get("outer_wall_speed").expect("present");
        assert_eq!(ow.value, "60", "the plate's own process wins");
        assert_eq!(ow.source_layer.as_deref(), Some("user"));
    }
}
