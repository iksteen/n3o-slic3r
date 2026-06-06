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
/// (not a heuristic): process fragment → the `"user"` row (labeled
/// "Profile" — the selected quality/process profile), nozzle fragment +
/// extruder-vector assembly → `"nozzle"`, bed → `"build_plate"`, filament →
/// `"filament"`, and `machine.toml` + synthesized machine-topology rules →
/// `"printer"`. Returns the frontend `CascadeLayer` id, or `None` for
/// `<plate-overrides>` (the panel draws override tiers itself) / anything
/// unrecognized.
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
    } else if s.contains("/nozzles/") || s.contains("<extruder-vector-assembly>") {
        Some("nozzle")
    } else if s.contains("machine.toml")
        || s.contains("<flush-defaults>")
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
    // Fragment-only resolution (no override tiers) — the panel draws
    // project/object rows from its own maps.
    let Some(resolved) = resolve_plate(p, plate_id, &BTreeMap::new())? else {
        return Ok(PlateResolvedJson {
            entries: HashMap::new(),
        });
    };
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

/// Compose + resolve a plate's cascade, folding `overrides` in as the
/// top-precedence layer exactly as the slice path folds
/// `Plate.project_overrides` (`slice::orchestrator::resolve_cascade`).
/// Pass an empty map for the fragment-only resolution the settings
/// ladder wants, or the plate's project overrides when the resolved
/// value has to match what actually slices (e.g. the priming-tower
/// position). Returns `None` for an unbound plate (no printer instance).
fn resolve_plate(
    p: &Project,
    plate_id: PlateId,
    overrides: &std::collections::BTreeMap<String, String>,
) -> Result<Option<crate::core::cascade::Resolved>, String> {
    let plate = p
        .plate(plate_id)
        .ok_or_else(|| format!("unknown plate id {plate_id:?}"))?;
    let Some(instance_id) = plate.printer_instance_id() else {
        return Ok(None);
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
    let cascade = crate::core::profile_library::compose_cascade(&effective, &[], overrides)
        .map_err(|e| format!("compose: {e}"))?;
    let ctx = crate::core::project::SlicingContext::new(
        std::sync::Arc::new(printer),
        std::sync::Arc::new(bed),
        filaments,
    );
    Ok(Some(crate::core::cascade::resolve(&cascade, &ctx)))
}

/// Resolved priming-tower placement + footprint for one plate, in bed
/// millimetres (world space — the bed's corner is the world origin).
/// `x`/`y` are the tower's lower-left corner (`wipe_tower_x/y`); `width`
/// is the square footprint (`prime_tower_width`); `brim` the skirt that
/// rings it; `rotation` is degrees about the tower (0 for both MVP
/// printers — carried for fidelity).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TowerGeometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub brim: f64,
    pub rotation: f64,
    /// Distinct material count this resolved against. The viewport pairs a
    /// sliced tower mesh with the count it was sliced at and treats the
    /// mesh as stale once this diverges (the only thing that reshapes the
    /// tower; moving it does not).
    pub material_count: usize,
    /// The plate's bound printer instance. The viewport also keys the cached
    /// tower mesh on this: a rebind to a different printer reshapes the tower
    /// (and doesn't re-slice), so the mesh must go stale even when the
    /// material count is unchanged. `None` only if the plate is unbound.
    pub printer_instance_id: Option<String>,
}

/// The active plate's priming-tower geometry for the viewport overlay,
/// or `None` when the plate is unbound or has no tower
/// (`enable_prime_tower` off). Visibility keys on `enable_prime_tower`,
/// not the purge-tower capability: both MVP printers run a tower (the
/// A1 mini purges through it, the U1 uses it for toolhead re-entry), and
/// only the purge-*volume* options are toolchanger-gated. The plate's
/// project overrides are folded in, so the box tracks exactly where the
/// tower slices — including a position the user has dragged it to.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub fn plate_tower_geometry(
    plate_id: PlateId,
    state: State<Arc<Mutex<Project>>>,
) -> Result<Option<TowerGeometry>, String> {
    let p = state.lock().map_err(|e| format!("project lock: {e}"))?;
    tower_geometry_for_plate(&p, plate_id)
}

/// Core of [`plate_tower_geometry`], split out for testing without a
/// Tauri `State`.
pub fn tower_geometry_for_plate(
    p: &Project,
    plate_id: PlateId,
) -> Result<Option<TowerGeometry>, String> {
    let Some(plate) = p.plate(plate_id) else {
        return Ok(None);
    };
    // A wipe/prime tower is only generated for a multi-material print —
    // ≥2 distinct filament indices among the plate's objects. With a single
    // material there are no tool changes, so libslic3r emits no tower
    // regardless of `enable_prime_tower`; the overlay must match. (Same
    // "referenced materials" notion the pre-slice gate uses:
    // `extruder_id.unwrap_or(1)`.)
    let distinct_materials: std::collections::HashSet<u8> = plate
        .scene
        .objects
        .values()
        .map(|o| o.extruder_id.unwrap_or(1))
        .collect();
    if distinct_materials.len() < 2 {
        return Ok(None);
    }
    // Fold the plate's project-tier overrides into the compose exactly as
    // the slice path does, so a dragged position resolves here too.
    let overrides: std::collections::BTreeMap<String, String> =
        plate.project_overrides.clone().into_iter().collect();
    let Some(resolved) = resolve_plate(p, plate_id, &overrides)? else {
        return Ok(None);
    };

    let enabled = resolved
        .get("enable_prime_tower")
        .map(|v| matches!(v.value.trim(), "1" | "true" | "True"))
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }

    // Cascade-resolved value if a fragment/override sets it, else the
    // engine's compiled default (the U1 pins no position, so its tower
    // sits at libslic3r's default until dragged).
    let num = |key: &str| -> Option<f64> {
        resolved
            .get(key)
            .map(|v| v.value.clone())
            .or_else(|| crate::core::cascade::engine_default_serialized(key))
            .and_then(|s| s.trim().parse::<f64>().ok())
    };

    Ok(Some(TowerGeometry {
        x: num("wipe_tower_x").unwrap_or(0.0),
        y: num("wipe_tower_y").unwrap_or(0.0),
        width: num("prime_tower_width").unwrap_or(0.0),
        brim: num("prime_tower_brim_width").unwrap_or(0.0),
        rotation: num("wipe_tower_rotation_angle").unwrap_or(0.0),
        material_count: distinct_materials.len(),
        printer_instance_id: plate.printer_instance_id().map(str::to_owned),
    }))
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
        // Bind the user's last-selected printer to the new project's plate.
        let preferred = crate::core::config::load().defaults.printer_instance;
        *p = Project::with_preferred_printer(preferred.as_deref());
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
        assert_eq!(p("<filament-topology>"), Some("printer"));
        assert_eq!(
            p("bbl/printer/bambu-lab-a1-mini/nozzles/0.4.toml"),
            Some("nozzle"),
            "nozzle fragment → its own Nozzle row, split out of Printer",
        );
        assert_eq!(p("<extruder-vector-assembly>"), Some("nozzle"));
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
        assert!(project.plates[0].printer_instance_id().is_some());

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

    /// Add a cube on the active plate assigned to `material` (its 1-based
    /// filament index). Two distinct materials make the plate multi-material
    /// — the condition a wipe/prime tower is generated for.
    fn add_cube(p: &mut Project, material: u8) {
        use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
        use crate::core::scene::transform::Transform;
        let mesh = p.register_mesh(NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: crate::core::printer::profile::BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        });
        p.register_object(NewSceneObject {
            mesh,
            transform: Transform::IDENTITY,
            name: format!("cube-m{material}"),
            visible: true,
            extruder_id: Some(material),
            parent: None,
            group_id: None,
        });
    }

    #[test]
    fn tower_geometry_for_a1_mini_reads_pinned_position_and_footprint() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        // Multi-material plate → the tower is generated.
        add_cube(&mut project, 1);
        add_cube(&mut project, 2);
        let bound = project.plates[0].printer_instance_id().map(str::to_owned);
        let t = tower_geometry_for_plate(&project, plate_id)
            .expect("ok")
            .expect("the A1 mini runs a prime tower for a multi-material plate");
        // Position pinned in machine.toml; footprint in the process fragment.
        assert_eq!(t.x, 5.0, "wipe_tower_x");
        assert_eq!(t.y, 130.0, "wipe_tower_y");
        assert_eq!(t.width, 35.0, "prime_tower_width");
        assert_eq!(t.brim, 3.0, "prime_tower_brim_width");
        // Carries the bound printer instance so the viewport can drop a cached
        // tower mesh on a rebind to a different printer (which reshapes the
        // tower without re-slicing).
        assert!(bound.is_some(), "default plate is auto-bound");
        assert_eq!(
            t.printer_instance_id, bound,
            "carries the bound instance id"
        );
    }

    #[test]
    fn tower_geometry_is_none_for_a_single_material_plate() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        // One material (or none) → no tool changes → no tower, even though
        // enable_prime_tower is set.
        add_cube(&mut project, 1);
        assert!(
            tower_geometry_for_plate(&project, plate_id)
                .expect("ok")
                .is_none(),
            "single-material plate must not show a tower",
        );
    }

    #[test]
    fn tower_geometry_tracks_a_project_override_position() {
        let _ = slic3r_ffi::init(None, 3);
        let mut project = Project::default();
        let plate_id = project.plates[0].id;
        add_cube(&mut project, 1);
        add_cube(&mut project, 2);
        // Dragging the tower writes a project-tier wipe_tower_x override;
        // the geometry the viewport reads must fold it in (so the box
        // tracks where the tower will actually slice).
        project
            .project_override_set(plate_id, "wipe_tower_x".into(), "42".into())
            .expect("set override");
        let t = tower_geometry_for_plate(&project, plate_id)
            .expect("ok")
            .expect("tower");
        assert_eq!(t.x, 42.0, "overridden position resolves here");
        assert_eq!(t.y, 130.0, "the untouched axis stays pinned");
    }

    #[test]
    fn plate_resolve_attributes_nozzle_keys_to_the_nozzle_layer() {
        // The nozzle fragment (via the extruder-vector assembly) is its own
        // ladder row, not folded into Printer. Check both a machine-bucket
        // key (`nozzle_diameter`, hidden in the panel) and a user-visible
        // one (`retraction_length`, shown under Retraction) so the row is
        // demonstrably reachable from the UI.
        let _ = slic3r_ffi::init(None, 3);
        let project = Project::default();
        let plate_id = project.plates[0].id;
        let resolved = resolve_plate_cascade(&project, plate_id).expect("resolve");
        for key in ["nozzle_diameter", "retraction_length"] {
            let e = resolved
                .entries
                .get(key)
                .unwrap_or_else(|| panic!("{key} resolved"));
            assert_eq!(e.source_layer.as_deref(), Some("nozzle"), "{key} → Nozzle");
        }
    }
}
