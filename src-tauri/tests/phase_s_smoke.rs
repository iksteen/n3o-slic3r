#![cfg(feature = "test-fixtures")]
//! Multi-printer project exit-criteria smoke.
//!
//! Locks the multi-instance + multi-filament work from PR-S-1
//! through PR-S-10 into CI. If the cascade composer regresses
//! filament fan-out, per-OptType separator dispatch, or
//! `flush_volumes_matrix` sizing, this fails loudly before a real
//! slice ever runs.
//!
//! Three legs from docs/dev/settings-model.md §11.4:
//!
//!   1. **Multi-filament A1 + AMS Lite** — slice a 4-color model on
//!      bambi (1 extruder × 5 AMS slots), verify ≥2 filaments tracked
//!      and ≥1 `M620 SnA` swap macro in the gcode.
//!   2. **Multi-instance project** — slice the same model on snappy
//!      (4-toolhead toolchanger), verify ≥2 filaments tracked and
//!      print-body `T<n>` changes emitted. Together with leg 1 this
//!      exercises the per-job `printer_instance_id` binding path.
//!   3. **Copy-vs-vendor binding** — deferred. The in-app
//!      filament/process copy mechanic is a tracked MVP exclusion
//!      (settings-model.md §9); covering it requires that surface to
//!      land first.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
// Mutex is for the per-test event-collecting sinks only; the FFI
// slice itself serializes inside `slic3r_ffi::slice`'s process-wide
// SLICE_LOCK, so no test-side lock needed.

use n3o_slic3r_lib::core::cascade::commands::{ContextJson, OverrideFileSpec};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::input::SliceObject;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, PlateSummary, SliceEvent, SliceJobInput,
};
use slic3r_ffi::init as ffi_init;

/// Resolve the plate's bound instance (test-fixtures registry) and run the
/// blocking slice — the pure orchestrator takes the instance as a parameter.
fn run_blocking(
    input: SliceJobInput,
    registry: &JobRegistry,
    sink: EventSink,
) -> Result<
    n3o_slic3r_lib::core::slice::JobId,
    n3o_slic3r_lib::core::slice::orchestrator::SliceStartError,
> {
    let instance = n3o_slic3r_lib::core::printer::lookup_instance(&input.printer_instance_id)
        .expect("test-fixtures: bound instance resolves");
    run_slice_job_blocking(input, &instance, registry, sink)
}

/// Build a slice input, resolving `plate_id`'s bound instance (the command
/// boundary does this in prod).
fn build_input(
    project: &n3o_slic3r_lib::core::project::Project,
    plate_id: n3o_slic3r_lib::core::project::PlateId,
    output_dir: String,
) -> Result<SliceJobInput, n3o_slic3r_lib::core::slice::input::SliceInputError> {
    let inst = project
        .plate(plate_id)
        .and_then(|p| p.printer_instance_id())
        .and_then(n3o_slic3r_lib::core::printer::lookup_instance);
    n3o_slic3r_lib::core::slice::input::build_slice_input(project, plate_id, output_dir, inst.as_ref())
}


/// Load every object of a `.3mf` into buffer-load [`SliceObject`]s for a
/// hand-built `SliceJobInput`, preserving per-object transform, extruder,
/// paint, overrides, and group. Mirrors what `build_slice_input` does for
/// a project's scene (minus the printer-instance extruder remap, which the
/// hand-built contexts don't apply).
fn objects_from_3mf(path: &std::path::Path) -> Vec<SliceObject> {
    let p = n3o_slic3r_lib::core::threemf::load_3mf(path).expect("load 3mf for slice objects");
    p.objects
        .iter()
        .map(|o| {
            let m = &p.meshes[o.mesh_idx];
            SliceObject {
                name: o.name.clone(),
                vertices: Arc::new(m.vertices.clone()),
                indices: Arc::new(m.indices.clone()),
                paint: m.paint_colors.clone().map(Arc::new),
                support_paint: m.support_paint.clone().map(Arc::new),
                transform: o.transform.matrix.map(f64::from),
                extruder: o.extruder_id.unwrap_or(1) as i32,
                overrides: o.overrides.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                group: o.group,
                group_overrides: vec![],
                modifiers: vec![],
            }
        })
        .collect()
}

static FFI_INIT: Once = Once::new();
fn ensure_ffi_init() {
    FFI_INIT.call_once(|| {
        ffi_init(None, 3).expect("slic3r_init");
    });
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// 4-color benchy AMS test model (CC BY-NC, attribution in
/// `examples/spike3/NOTICE.md`). Used as the multi-color fixture
/// for both legs.
fn fourcolor_3mf() -> PathBuf {
    workspace_root().join("examples/spike3/fourcolor.3mf")
}

/// OrcaCube v2 — a real multi-volume `.3mf` (cube + plug) used here as a
/// geometry fixture. Lives under the crate's test fixtures.
fn orca_cube_3mf() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/3mf/orca-cube-v2.3mf")
}

/// A single 20mm cube exported as two foreign 3MFs with identical geometry —
/// one plain, one carrying an object-scoped `layer_height = 0.25` override (as a
/// foreign Orca export stores it). Authored by
/// `tests/fixtures/3mf/cube-override-pair.py`.
fn cube_3mf(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/3mf/{name}"))
}

fn canonical_plate() -> BuildPlate {
    BuildPlate {
        identity: "Textured PEI".into(),
        libslic3r_curr_bed_type: "Textured PEI Plate".into(),
    }
}

fn canonical_filament() -> FilamentProfile {
    FilamentProfile {
        identity: "Generic PLA".into(),
        base_type: "PLA".into(),
        vendor: None,
        color: None,
    }
}

fn bambi_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu Lab A1 mini".into(),
        supported_build_plates: vec![
            "Cool".into(),
            "Textured PEI".into(),
            "Smooth PEI".into(),
            "Engineering".into(),
            "SuperTack".into(),
        ],
        toolheads: vec![Toolhead {
            default_nozzle_diameter: "0.4".into(),
            hotend_type: "stainless_steel".into(),
            max_temp: 300.0,
        }],
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [180.0, 180.0, 180.0],
        },
        exclusion_zones: vec![],
        ..Default::default()
    }
}

fn snappy_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Snapmaker U1".into(),
        supported_build_plates: vec!["Textured PEI Plate".into()],
        toolheads: (0..4)
            .map(|_| Toolhead {
                default_nozzle_diameter: "0.4".into(),
                hotend_type: "hardened_steel".into(),
                max_temp: 300.0,
            })
            .collect(),
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [220.0, 220.0, 220.0],
        },
        exclusion_zones: vec![],
        ..Default::default()
    }
}

fn bender_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Creality Ender-3 S1".into(),
        supported_build_plates: vec!["Textured PEI".into(), "PC Spring Steel".into()],
        toolheads: vec![Toolhead {
            default_nozzle_diameter: "0.4".into(),
            hotend_type: "brass".into(),
            max_temp: 260.0,
        }],
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [220.0, 220.0, 270.0],
        },
        exclusion_zones: vec![],
        ..Default::default()
    }
}

fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<SliceEvent>>>) {
    let bucket: Arc<Mutex<Vec<SliceEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let bucket_for_cb = bucket.clone();
    let sink: EventSink = Box::new(move |event| {
        bucket_for_cb.lock().unwrap().push(event);
    });
    (sink, bucket)
}

/// Slice `examples/spike3/fourcolor.3mf` on the given instance and
/// return the produced gcode path + summary. `filaments_in_context`
/// is the length of the predicate-side `ContextJson.filaments` —
/// the composer pulls real per-slot identities off the bound
/// instance regardless, but the orchestrator still reads this list
/// for predicate matching.
fn slice_fourcolor(
    instance_id: &str,
    printer: PrinterProfile,
    filaments_in_context: usize,
) -> (PathBuf, PlateSummary) {
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    let temp_dir = std::env::temp_dir().join(format!(
        "n3o-phase-s-smoke-{}-{}",
        instance_id,
        std::process::id(),
    ));

    let objects = objects_from_3mf(&fourcolor_3mf());
    // Everything slices in-memory, groups included (add_group + add_volume).
    let input = SliceJobInput {
        objects,
        output_dir: temp_dir.display().to_string(),
        context: ContextJson {
            printer,
            plate: canonical_plate(),
            filaments: (0..filaments_in_context)
                .map(|_| canonical_filament())
                .collect(),
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![OverrideFileSpec {
                label: "phase-s-smoke".into(),
                // The 3DBenchy has floating regions (the bow overhang)
                // and refuses to slice without supports. Enable tree
                // supports for both legs so the fixture is sliceable
                // on any printer instance without per-instance tuning.
                content: "enable_support = \"1\"\nsupport_type = \"tree_auto\"\n".into(),
            }],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids: vec![1],
        printer_instance_id: instance_id.into(),
        material_layout: vec![],
        quality_profile: None,
        paint_filament_remap: None,
    };

    run_blocking(input, &registry, sink)
        .unwrap_or_else(|e| panic!("{instance_id}: synchronous start failed: {e}"));

    let events = events.lock().unwrap();
    // Surface the failure cause cleanly instead of unwrapping past
    // a JobFailed event.
    if let Some(SliceEvent::JobFailed { error, .. }) = events
        .iter()
        .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        panic!("{instance_id}: slice failed: {error:?}");
    }
    let (path, summary) = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished {
                output_path,
                summary,
                ..
            } => Some((PathBuf::from(output_path), summary.clone())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{instance_id}: no PlateFinished in event stream"));

    assert!(
        path.exists(),
        "{instance_id}: gcode missing at {}",
        path.display()
    );
    (path, summary)
}

/// Leg 1: multi-filament A1 + AMS Lite.
///
/// 4-color benchy on the bambi PrinterInstance (1 extruder × 5 AMS
/// slots). The composer fans the filament fragments and synthesizes
/// the colour/flush vectors so libslic3r sees a length-5 filament
/// dimension. Verifies ≥2 filaments tracked in the summary and ≥1
/// `M620 SnA` AMS swap macro in the gcode body.
#[test]
fn bambi_multi_color_slices_with_filament_tracking() {
    ensure_ffi_init();

    let (gcode_path, summary) = slice_fourcolor("bambi", bambi_printer(), 5);

    let used = summary
        .filament_used_grams
        .values()
        .filter(|v| **v > 0.0)
        .count();
    assert!(
        used >= 2,
        "expected ≥2 non-zero filament entries (multi-color tracking), got {used}: {:?}",
        summary.filament_used_grams,
    );

    // `M620 SnA` is Bambu's "swap material to AMS slot n" marker —
    // one per filament change in the print body.
    let gcode = std::fs::read_to_string(&gcode_path).expect("read bambi gcode");
    let swaps = gcode
        .lines()
        .filter(|l| {
            let l = l.trim_start();
            l.starts_with("M620 S") && l.ends_with('A')
        })
        .count();
    assert!(
        swaps >= 1,
        "expected ≥1 M620 SnA AMS swap macro in bambi gcode, got {swaps}",
    );

    let _ = std::fs::remove_dir_all(gcode_path.parent().unwrap());
}

/// Leg 2: multi-instance project.
///
/// Same 4-color model on snappy (4-toolhead toolchanger). Asserts
/// the per-job `printer_instance_id` binding routes through the
/// orchestrator correctly and that the U1's toolchanger emits
/// `T<n>` changes in the print body. Together with leg 1 this
/// proves the multi-instance project shape (different plates →
/// different printer_instance_id) holds end-to-end.
#[test]
fn snappy_multi_color_slices_with_toolhead_changes() {
    ensure_ffi_init();

    let (gcode_path, summary) = slice_fourcolor("snappy", snappy_printer(), 4);

    let used = summary
        .filament_used_grams
        .values()
        .filter(|v| **v > 0.0)
        .count();
    assert!(
        used >= 2,
        "expected ≥2 non-zero filament entries (multi-color tracking), got {used}: {:?}",
        summary.filament_used_grams,
    );

    // Bare `T<n>` lines are real toolhead docks on the U1.
    // Setup-area T0 lives in start_gcode; print-body changes are
    // additional. We want ≥4 total (start_gcode 3× T0 + ≥1 body
    // change), but in practice the U1's body has multiple cycles.
    let gcode = std::fs::read_to_string(&gcode_path).expect("read snappy gcode");
    let t_changes = gcode
        .lines()
        .filter(|l| {
            let l = l.trim();
            l.len() >= 2
                && l.len() <= 4
                && l.starts_with('T')
                && l[1..].chars().all(|c| c.is_ascii_digit())
        })
        .count();
    assert!(
        t_changes >= 4,
        "expected ≥4 T<n> toolhead changes in snappy gcode (setup + body), got {t_changes}",
    );

    let _ = std::fs::remove_dir_all(gcode_path.parent().unwrap());
}

/// A multi-material model on **Bender** (Ender-3 S1: single extruder, no AMS,
/// not a toolchanger, not firmware-routed) flattens onto the one toolhead —
/// the gcode never emits a bare `T<n>` for `n ≥ 1`. The four declared
/// materials all auto-bind to Bender's single slot, so there's nowhere else
/// to route. Contrast [`snappy_multi_color_slices_with_toolhead_changes`],
/// where the U1's four toolheads produce real `T<n>` docks.
#[test]
fn bender_flattens_multi_material_to_single_toolhead() {
    ensure_ffi_init();

    let (gcode_path, _summary) = slice_fourcolor("bender", bender_printer(), 1);

    // Bare `T<n>` lines are toolhead selections. On a single-extruder printer
    // there is only T0, so any `T1`/`T2`/… means the multi-material model
    // didn't collapse onto the one toolhead.
    let gcode = std::fs::read_to_string(&gcode_path).expect("read bender gcode");
    let higher_toolheads: Vec<String> = gcode
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| {
            l.len() >= 2
                && l.len() <= 4
                && l.starts_with('T')
                && l[1..].chars().all(|c| c.is_ascii_digit())
                && l != "T0"
        })
        .collect();
    assert!(
        higher_toolheads.is_empty(),
        "single-extruder Bender must flatten multi-material to T0 only; got {higher_toolheads:?}",
    );

    let _ = std::fs::remove_dir_all(gcode_path.parent().unwrap());
}

/// Multi-volume groups round-trip through the slice pipeline as one
/// ModelObject, not as N freestanding objects. Loads
/// `cube-halves-2mat.3mf` (BBS `<components>` + per-`<part>` extruder
/// hints) on bambi, slices it, and asserts:
///
/// 1. No "floating regions" warning fired — the upper half's bottom
///    face sits on the lower half's top face, which is what libslic3r
///    must see when the two are volumes of one ModelObject. Without
///    group preservation in the writer, libslic3r would treat the
///    upper half as standalone at world Z=10..20 with nothing under
///    it, fire `SharpTail`/`floating regions`, and surface
///    "It seems object Upper half (M2) has floating regions" — the
///    bug this guards against.
/// 2. The gcode emits ≥1 AMS swap macro between M1 and M2 — proves
///    the per-volume extruder hints survived the round-trip into
///    libslic3r's per-region planning.
#[test]
fn cube_halves_slices_as_one_multivolume_object_no_floating_warning() {
    use n3o_slic3r_lib::core::project::{PlateId, Project};
    use n3o_slic3r_lib::core::scene::state::NewSceneObject;
    use n3o_slic3r_lib::core::threemf::load_3mf;

    ensure_ffi_init();

    let fixture = workspace_root().join("src-tauri/tests/fixtures/3mf/cube-halves-2mat.3mf");
    let project_3mf = load_3mf(&fixture).expect("load cube-halves fixture");

    let mut project = Project::default();
    project.plates[0].set_printer(Some("bambi".into()));
    let mesh_ids: Vec<_> = project_3mf
        .meshes
        .into_iter()
        .map(|m| project.register_mesh(m))
        .collect();
    for obj in project_3mf.objects {
        project.register_object(NewSceneObject {
            mesh: mesh_ids[obj.mesh_idx],
            transform: obj.transform,
            name: obj.name,
            visible: true,
            extruder_id: obj.extruder_id,
            group: obj.group,
        }, n3o_slic3r_lib::core::printer::lookup_instance("bambi").as_ref());
    }

    let temp_dir = std::env::temp_dir().join(format!("n3o-cube-halves-{}", std::process::id(),));
    let input = build_input(&project, PlateId(1), temp_dir.display().to_string())
        .expect("build_slice_input");

    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_blocking(input, &registry, sink).expect("synchronous start");

    let events = events.lock().unwrap();
    if let Some(SliceEvent::JobFailed { error, .. }) = events
        .iter()
        .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        panic!("slice failed: {error:?}");
    }

    // No floating-regions warning should fire. The orchestrator
    // surfaces libslic3r warnings on PlateProgress events with
    // negative percent — scan for the substring directly.
    let floating_warning = events.iter().find(|e| match e {
        SliceEvent::PlateProgress { stage, .. } => stage.contains("floating regions"),
        _ => false,
    });
    assert!(
        floating_warning.is_none(),
        "libslic3r emitted a floating-regions warning — group preservation \
         broke and the upper half is being treated as a freestanding object \
         (warning was: {floating_warning:?})",
    );

    let gcode_path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(PathBuf::from(output_path)),
            _ => None,
        })
        .expect("PlateFinished");
    let gcode = std::fs::read_to_string(&gcode_path).expect("read gcode");

    // The two volumes use materials 1 and 2 — libslic3r emits an AMS
    // swap macro (`M620 S<N>A`) when transitioning between them.
    let swap_count = gcode.matches("M620").count();
    assert!(
        swap_count >= 1,
        "expected ≥1 `M620` AMS swap in the gcode for a 2-material print, got {swap_count}",
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Per-object setting overrides reach the engine. Slices OrcaCube on
/// bambi twice — once unmodified, once with an object-scoped
/// `layer_height` override — and asserts the override drops the sliced
/// layer count. Proves the whole chain end-to-end: the scope gate keeps
/// the key, the writer emits it as object metadata in the temp `.3mf`,
/// and libslic3r folds it into `ModelObject::config` on load (the same
/// channel per-object `extruder` rides). `layer_count` comes from the
/// gcode footer the summary parser reads, so this is a G-code assertion.
#[test]
fn object_layer_height_override_changes_sliced_layer_count() {
    use n3o_slic3r_lib::core::project::{PlateId, Project};
    use n3o_slic3r_lib::core::scene::state::NewSceneObject;
    use n3o_slic3r_lib::core::threemf::load_3mf;

    ensure_ffi_init();
    let cube_path = orca_cube_3mf();

    // Build a fresh bambi project from OrcaCube; return it + the object ids.
    let build = || {
        let project_3mf = load_3mf(&cube_path).expect("load OrcaCube");
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()));
        let mesh_ids: Vec<_> = project_3mf
            .meshes
            .into_iter()
            .map(|m| project.register_mesh(m))
            .collect();
        let obj_ids: Vec<_> = project_3mf
            .objects
            .into_iter()
            .map(|obj| {
                project.register_object(NewSceneObject {
                    mesh: mesh_ids[obj.mesh_idx],
                    transform: obj.transform,
                    name: obj.name,
                    visible: true,
                    extruder_id: obj.extruder_id,
                    group: obj.group,
                }, n3o_slic3r_lib::core::printer::lookup_instance("bambi").as_ref())
            })
            .collect();
        (project, obj_ids)
    };

    let slice_layer_count = |project: &Project, tag: &str| -> u32 {
        let temp_dir =
            std::env::temp_dir().join(format!("n3o-objovr-{tag}-{}", std::process::id()));
        let input =
            build_input(project, PlateId(1), temp_dir.display().to_string())
                .expect("build_slice_input");
        let registry = JobRegistry::new();
        let (sink, events) = collecting_sink();
        run_blocking(input, &registry, sink).expect("synchronous start");
        let events = events.lock().unwrap();
        if let Some(SliceEvent::JobFailed { error, .. }) = events
            .iter()
            .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
        {
            panic!("[{tag}] slice failed: {error:?}");
        }
        let layers = events
            .iter()
            .find_map(|e| match e {
                SliceEvent::PlateFinished { summary, .. } => Some(summary.layer_count),
                _ => None,
            })
            .unwrap_or_else(|| panic!("[{tag}] no PlateFinished"));
        let _ = std::fs::remove_dir_all(&temp_dir);
        layers
    };

    // Baseline: the profile default layer height (0.2mm).
    let (base_project, _) = build();
    let base_layers = slice_layer_count(&base_project, "base");

    // Override each object's layer_height to a taller value (still within
    // the 0.4mm nozzle's range) → meaningfully fewer layers.
    let (mut ovr_project, obj_ids) = build();
    for id in &obj_ids {
        ovr_project
            .object_override_set(PlateId(1), *id, "layer_height".into(), "0.25".into())
            .expect("set object override");
    }
    let ovr_layers = slice_layer_count(&ovr_project, "override");

    assert!(
        base_layers > 0 && ovr_layers > 0,
        "both slices must report a layer count (base={base_layers}, override={ovr_layers})",
    );
    assert!(
        ovr_layers < base_layers,
        "per-object layer_height override didn't reach the engine: a taller layer height must \
         yield fewer layers, but base={base_layers} and override={ovr_layers}",
    );
}

/// Full import→slice round-trip. A 3MF that *carries* a per-object
/// `layer_height` override in its `model_settings.config` (exactly how a
/// foreign Orca export stores it) is loaded the way the app's
/// `scene_load_3mf` does — `load_3mf` + `register_object` +
/// `apply_imported_object_overrides` — then sliced. Asserts (a) the import
/// populated `scene.object_overrides`, and (b) the imported override drops
/// the sliced layer count vs. the same geometry imported without it.
/// Covers the read seam the other tests exercise only piecewise:
/// bbs_meta → apply_bbs_metadata → apply_imported_object_overrides →
/// build_plate_geometry → libslic3r → G-code.
#[test]
fn imported_object_override_reaches_the_engine_end_to_end() {
    use n3o_slic3r_lib::core::project::{PlateId, Project};
    use n3o_slic3r_lib::core::scene::state::NewSceneObject;
    use n3o_slic3r_lib::core::threemf::load_3mf;
    use std::path::Path;

    ensure_ffi_init();
    let tmp = std::env::temp_dir();
    let pid = std::process::id();

    // Two foreign 3MFs with identical geometry: one plain, one carrying a
    // per-object layer_height override at the <object> level of
    // model_settings.config (how a foreign Orca export records it). Authored by
    // tests/fixtures/3mf/cube-override-pair.py.
    let base_3mf = cube_3mf("cube-plain.3mf");
    let ovr_3mf = cube_3mf("cube-override.3mf");

    // Import a 3MF into a fresh bambi project exactly as scene_load_3mf does.
    let import = |path: &Path| -> Project {
        let p3mf = load_3mf(path).expect("load");
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()));
        let mesh_ids: Vec<_> = p3mf
            .meshes
            .into_iter()
            .map(|m| project.register_mesh(m))
            .collect();
        for obj in &p3mf.objects {
            let id = project.register_object(NewSceneObject {
                mesh: mesh_ids[obj.mesh_idx],
                transform: obj.transform,
                name: obj.name.clone(),
                visible: true,
                extruder_id: obj.extruder_id,
                group: obj.group,
            }, n3o_slic3r_lib::core::printer::lookup_instance("bambi").as_ref());
            project.apply_imported_object_overrides(id, &obj.overrides);
        }
        project
    };

    let slice_layers = |project: &Project, tag: &str| -> u32 {
        let out = tmp.join(format!("n3o-imp-slice-{tag}-{pid}"));
        let input = build_input(project, PlateId(1), out.display().to_string())
            .expect("build_slice_input");
        let registry = JobRegistry::new();
        let (sink, events) = collecting_sink();
        run_blocking(input, &registry, sink).expect("synchronous start");
        let events = events.lock().unwrap();
        if let Some(SliceEvent::JobFailed { error, .. }) = events
            .iter()
            .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
        {
            panic!("[{tag}] slice failed: {error:?}");
        }
        let layers = events
            .iter()
            .find_map(|e| match e {
                SliceEvent::PlateFinished { summary, .. } => Some(summary.layer_count),
                _ => None,
            })
            .unwrap_or_else(|| panic!("[{tag}] no PlateFinished"));
        let _ = std::fs::remove_dir_all(&out);
        layers
    };

    let base = import(&base_3mf);
    let ovr = import(&ovr_3mf);

    // (a) The import actually populated object_overrides on the override 3MF
    //     and left the plain one clean.
    let ovr_count: usize = ovr
        .plate(PlateId(1))
        .unwrap()
        .scene
        .object_overrides
        .values()
        .filter(|m| m.get("layer_height").map(String::as_str) == Some("0.25"))
        .count();
    assert!(
        ovr_count >= 1,
        "import must populate scene.object_overrides with the file's layer_height override",
    );
    assert!(
        base.plate(PlateId(1))
            .unwrap()
            .scene
            .object_overrides
            .is_empty(),
        "plain import must carry no object overrides",
    );

    // (b) The imported override reaches the engine: fewer layers.
    let base_layers = slice_layers(&base, "base");
    let ovr_layers = slice_layers(&ovr, "ovr");
    assert!(
        base_layers > 0 && ovr_layers > 0,
        "both slices report a layer count (base={base_layers}, ovr={ovr_layers})",
    );
    assert!(
        ovr_layers < base_layers,
        "imported per-object layer_height override didn't reach the engine: base={base_layers}, \
         override={ovr_layers}",
    );
}

/// Pins the group transform-composition ORDER. `add_volume` does
/// `set_transformation(world * centering)` — matching the `.3mf` loader's
/// component path — where `centering` is the compensation `add_volume` bakes
/// when it re-centers the mesh. A member ROTATION does not commute with that
/// centering, so a flipped composition relocates the rotated member. Layer count
/// can't see it (same orientation either way); position can.
///
/// Oracle without a `.3mf` round-trip: slice the rotated member two ways — once
/// via the volume transform, once with that transform baked into the mesh
/// vertices and an identity transform. The baked path's identity transform
/// commutes with centering, so it's order-insensitive (the oracle); only the
/// via-transform path can move under a flipped composition. Same world geometry
/// either way → identical brim/skirt/prime → the print bounding boxes must
/// match. (Replaces the order coverage the deleted temp-`.3mf` parity test held.)
#[test]
fn grouped_member_rotation_composes_in_world_space() {
    use n3o_slic3r_lib::core::project::{PlateId, Project};
    use n3o_slic3r_lib::core::scene::primitives::{generate, PrimitiveKind, PrimitiveParams};
    use n3o_slic3r_lib::core::scene::state::{GroupId, NewMesh, NewSceneObject};
    use n3o_slic3r_lib::core::scene::transform::Transform;
    use glam::Vec3;

    ensure_ffi_init();

    // A 30×10×10 cube shifted off-origin so its local centroid is NOT at the
    // origin — that makes `add_volume`'s centering compensation non-trivial, so
    // the composition order actually matters (a centered mesh would commute).
    // (`bounding_box` stays at generate()'s value; the slice path reads only
    // vertices/indices, and register_object doesn't re-seat here.)
    let mut params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
    params.width = 30.0;
    params.depth = 10.0;
    params.height = 10.0;
    let mut mesh = generate(PrimitiveKind::Cube, params);
    for c in mesh.vertices.chunks_exact_mut(3) {
        c[0] += 15.0; // → local X 0..30
        c[1] += 5.0; //  → local Y 0..10
        c[2] += 5.0; //  → local Z 0..10 (sits on the bed)
    }

    // Anchor member (keeps it a 2-member group → the add_volume path, not the
    // single-volume add_object collapse) + a rotated member.
    let anchor = Transform::translation(Vec3::new(40.0, 80.0, 0.0));
    let rotated = Transform::translation(Vec3::new(100.0, 50.0, 0.0))
        .compose(Transform::rotation_around(Vec3::Z, std::f32::consts::FRAC_PI_2));

    // Same rotation+placement baked straight into the vertices.
    let mut baked = mesh.clone();
    for c in baked.vertices.chunks_exact_mut(3) {
        let p = rotated.apply_point(Vec3::new(c[0], c[1], c[2]));
        c[0] = p.x;
        c[1] = p.y;
        c[2] = p.z;
    }

    // XY range of the G-code's extruding moves (the deposited footprint). The
    // header `min_x`/`max_x` comments aren't emitted for this profile, so read
    // the toolpath directly. Prime line + brim are identical between the two
    // slices (same geometry), so they cancel in the comparison.
    fn gcode_xy_bbox(path: &str) -> ([f32; 2], [f32; 2]) {
        let text = std::fs::read_to_string(path).expect("read gcode");
        let mut min = [f32::MAX; 2];
        let mut max = [f32::MIN; 2];
        for line in text.lines() {
            if !line.starts_with("G1 ") || !line.contains(" E") {
                continue; // extruding moves only (skip travels / retractions)
            }
            for tok in line.split_whitespace() {
                let axis = match tok.as_bytes().first() {
                    Some(b'X') => 0,
                    Some(b'Y') => 1,
                    _ => continue,
                };
                if let Ok(v) = tok[1..].parse::<f32>() {
                    min[axis] = min[axis].min(v);
                    max[axis] = max[axis].max(v);
                }
            }
        }
        assert!(min[0] <= max[0], "no extruding moves found in {path}");
        (min, max)
    }

    // Slice a 2-member group (anchor + `b`) and return the extruded XY bbox.
    let slice_bbox = |b_mesh: NewMesh, b_xform: Transform, tag: &str| -> ([f32; 2], [f32; 2]) {
        let mut project = Project::default();
        project.plates[0].set_printer(Some("bambi".into()));
        let g = GroupId::fresh();
        let m_anchor = project.register_mesh(mesh.clone());
        let m_b = project.register_mesh(b_mesh);
        let new = |m, t, name: &str| NewSceneObject {
            mesh: m,
            transform: t,
            name: name.into(),
            visible: true,
            extruder_id: None,
            group: Some(g),
        };
        project.register_object(new(m_anchor, anchor, "anchor"), n3o_slic3r_lib::core::printer::lookup_instance("bambi").as_ref());
        project.register_object(new(m_b, b_xform, "b"), n3o_slic3r_lib::core::printer::lookup_instance("bambi").as_ref());

        let out = std::env::temp_dir().join(format!("n3o-compose-{tag}-{}", std::process::id()));
        let input = build_input(&project, PlateId(1), out.display().to_string())
            .expect("build_slice_input");
        let registry = JobRegistry::new();
        let (sink, events) = collecting_sink();
        run_blocking(input, &registry, sink).expect("synchronous start");
        let events = events.lock().unwrap();
        if let Some(SliceEvent::JobFailed { error, .. }) =
            events.iter().find(|e| matches!(e, SliceEvent::JobFailed { .. }))
        {
            panic!("[{tag}] slice failed: {error:?}");
        }
        let path = events
            .iter()
            .find_map(|e| match e {
                SliceEvent::PlateFinished { output_path, .. } => Some(output_path.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("[{tag}] no PlateFinished"));
        let bbox = gcode_xy_bbox(&path);
        let _ = std::fs::remove_dir_all(&out);
        bbox
    };

    let (via_min, via_max) = slice_bbox(mesh.clone(), rotated, "via-transform");
    let (baked_min, baked_max) = slice_bbox(baked, Transform::IDENTITY, "baked");

    // Same world geometry both ways → identical footprint (≤1mm slack).
    // A flipped composition shifts the via-transform member by
    // centroid − Rz90(centroid) = (20mm in X, 10mm in Y) — far above the slack.
    for a in 0..2 {
        assert!(
            (via_min[a] - baked_min[a]).abs() < 1.0 && (via_max[a] - baked_max[a]).abs() < 1.0,
            "group composition order is wrong on axis {a}: via-transform XY bbox \
             [{:?},{:?}] != baked-geometry XY bbox [{:?},{:?}]",
            via_min,
            via_max,
            baked_min,
            baked_max,
        );
    }
}

/// MMU color-painting import → multi-material slice (Phase 1, AMS).
///
/// spinning-top.3mf is a single-extruder model whose 2nd filament is applied
/// by per-face `paint_color`, not a per-object `extruder`. Import it (binds to
/// bambi / A1 mini, AMS) and confirm the slice is genuine 2-material — the
/// painted faces reached filament 2. Exercises the whole chain: paint
/// round-trip into the slice 3MF + the painted-filament accounting + the
/// composer fanning `filament_colour` to the bound slot count (so MMU's
/// per-colour vector matches `filament_diameter`; a length mismatch crashes
/// libslic3r's segmentation).
///
/// INTERIM FIXTURE: a top-level *untracked* local file, so the test skips when
/// absent (e.g. CI) rather than failing. Replace with a committed, licensed
/// painted fixture once the paint work (incl. the U1 toolchanger path) lands.
#[test]
fn imported_painted_model_slices_as_multi_material() {
    use n3o_slic3r_lib::core::orca_import::import;

    ensure_ffi_init();
    let fixture = workspace_root().join("spinning-top.3mf");
    if !fixture.exists() {
        eprintln!(
            "skipping imported_painted_model_slices_as_multi_material: {} absent (interim local fixture)",
            fixture.display()
        );
        return;
    }

    let (project, _report) = import(&fixture).expect("import painted project");
    let pid = project.plates[0].id;
    let temp_dir = std::env::temp_dir().join(format!("n3o-paint-mm-{}", std::process::id()));
    let input = build_input(&project, pid, temp_dir.display().to_string())
        .expect("build_slice_input");

    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_blocking(input, &registry, sink).expect("synchronous start");
    let events = events.lock().unwrap();
    if let Some(SliceEvent::JobFailed { error, .. }) = events
        .iter()
        .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        panic!("painted slice failed: {error:?}");
    }
    let (path, summary) = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished {
                output_path,
                summary,
                ..
            } => Some((PathBuf::from(output_path), summary.clone())),
            _ => None,
        })
        .expect("PlateFinished");

    // ≥2 filaments actually consumed → the painted 2nd material reached the
    // engine (a 1-material slice would report a single extruder slot).
    assert!(
        summary.filament_used_grams.len() >= 2,
        "painted model must slice multi-material (>=2 filaments), got {:?}",
        summary.filament_used_grams,
    );
    // And an AMS swap macro between the two colors.
    let gcode = std::fs::read_to_string(&path).expect("read gcode");
    assert!(
        gcode.matches("M620").count() >= 1,
        "expected >=1 AMS swap (M620) between the painted colors",
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// MMU color-painting on a toolchanger → painted faces route to the bound
/// toolhead (Phase 2, U1).
///
/// Same painted fixture, rebound to snappy (Snapmaker U1, 4-toolhead
/// toolchanger). The base material binds to toolhead 1 and the painted
/// material to toolhead 3 — a *non-sequential* binding, so the paint state's
/// libslic3r filament index (2) must be remapped to the bound toolhead's flat
/// slot index (3). The orchestrator applies that remap to the loaded model via
/// `Model::remap_paint_filaments`; without it the painted faces would print on
/// toolhead 2 (filament index 2), not 3.
///
/// Verified at the G-code: the only tool-change commands are `T0` (base →
/// toolhead 1) and `T2` (painted → toolhead 3, 0-based). No `T1`/`T3` in the
/// print moves. This also covers the toolchanger MMU config invariant — the
/// composer fans `filament_colour` to the 4 slots so it matches the slot-fanned
/// `filament_diameter` (a shorter colour vector segfaults the segmentation).
///
/// INTERIM FIXTURE: shares spinning-top.3mf with the AMS test; skips when
/// absent. Replace with a committed, licensed painted fixture once the paint
/// work lands.
#[test]
fn imported_painted_model_routes_to_bound_toolhead_on_u1() {
    use n3o_slic3r_lib::core::orca_import::import;
    use n3o_slic3r_lib::core::printer::SlotRef;

    ensure_ffi_init();
    let fixture = workspace_root().join("spinning-top.3mf");
    if !fixture.exists() {
        eprintln!(
            "skipping imported_painted_model_routes_to_bound_toolhead_on_u1: {} absent (interim local fixture)",
            fixture.display()
        );
        return;
    }

    let (mut project, _report) = import(&fixture).expect("import painted project");
    // Rebind to the U1 toolchanger; its bundled process differs, so clear the
    // imported A1-mini process and let the instance default resolve.
    project.plates[0].set_printer(Some("snappy".into()));
    project.plates[0].quality_profile = None;
    // Non-sequential: base material 1 → toolhead 1, painted material 2 →
    // toolhead 3. The remap must follow the binding, not assume identity.
    project.plates[0].material_to_slot.insert(
        1,
        SlotRef {
            extruder: 0,
            slot: 0,
        },
    );
    project.plates[0].material_to_slot.insert(
        2,
        SlotRef {
            extruder: 2,
            slot: 0,
        },
    );

    let pid = project.plates[0].id;
    let temp_dir = std::env::temp_dir().join(format!("n3o-paint-u1-{}", std::process::id()));
    let input = build_input(&project, pid, temp_dir.display().to_string())
        .expect("build_slice_input");
    // build_slice_input computes the toolchanger paint remap; for this binding
    // the painted state 2 must route to flat slot index 3.
    assert_eq!(
        input.paint_filament_remap.as_deref(),
        Some([0, 1, 3].as_slice()),
        "painted state 2 should remap to the bound toolhead's flat slot index 3",
    );

    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_blocking(input, &registry, sink).expect("synchronous start");
    let events = events.lock().unwrap();
    if let Some(SliceEvent::JobFailed { error, .. }) = events
        .iter()
        .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        panic!("U1 painted slice failed: {error:?}");
    }
    let path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(PathBuf::from(output_path)),
            _ => None,
        })
        .expect("PlateFinished");

    // The print moves use exactly two toolheads: T0 (base → toolhead 1) and
    // T2 (painted → toolhead 3, 0-based). A bare `T<n>` on its own line is a
    // tool change; collect the distinct set.
    let gcode = std::fs::read_to_string(&path).expect("read gcode");
    let mut tools: Vec<u32> = gcode
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            t.strip_prefix('T')
                .filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
                .and_then(|rest| rest.parse::<u32>().ok())
        })
        .collect();
    tools.sort_unstable();
    tools.dedup();
    assert_eq!(
        tools,
        vec![0, 2],
        "painted faces must route to the bound toolheads T0 (base) + T2 (painted), got {tools:?}",
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// Per-triangle paint decode against real data (viewport display).
///
/// Imports spinning-top and decodes every painted mesh's per-triangle
/// `paint_color` to a dominant filament state. Validates the codec port on
/// real data — including the dozen recursively-split triangles whose long
/// hex strings the synthetic unit tests can't cover: every decoded state
/// must be a real filament (0 = base, or 1/2, the project's two filaments),
/// and BOTH painted filaments must appear. A codec desync (wrong reversal,
/// bit order, or split recursion) would surface as out-of-range states.
#[test]
fn decoded_paint_states_are_real_filaments_on_real_data() {
    use n3o_slic3r_lib::core::orca_import::import;
    use n3o_slic3r_lib::core::threemf::decode_dominant_states;

    let fixture = workspace_root().join("spinning-top.3mf");
    if !fixture.exists() {
        eprintln!(
            "skipping decoded_paint_states_are_real_filaments_on_real_data: {} absent",
            fixture.display()
        );
        return;
    }
    let (project, _report) = import(&fixture).expect("import painted project");
    let mut saw_painted_mesh = false;
    for mesh in project.meshes.values() {
        let Some(paint) = &mesh.paint_colors else {
            continue;
        };
        let Some(states) = decode_dominant_states(paint) else {
            continue;
        };
        saw_painted_mesh = true;
        assert_eq!(states.len(), paint.len(), "one state per triangle");
        assert!(
            states.iter().all(|&s| s <= 2),
            "every dominant state is a real filament (0/1/2); a higher value means a codec desync",
        );
        assert!(states.contains(&1), "filament 1 painted faces present");
        assert!(states.contains(&2), "filament 2 painted faces present");
    }
    assert!(saw_painted_mesh, "expected at least one painted mesh");
}

/// Manual tree supports: painted enforcers reach the engine and grow support
/// only where painted (the whole point of `support_type = tree(manual)`).
///
/// An L-shaped bracket (pillar on the bed + a horizontal arm cantilevered out
/// at the top) is sliced twice on bambi under `enable_support = 1` +
/// `support_type = tree(manual)`. The ONLY difference between the two runs is
/// the mesh's `support_paint`:
///   - painted: the arm's downward-facing underside is enforcer-painted, so
///     tree support grows under it down to the bed;
///   - unpainted: no enforcers → manual mode grows nothing.
/// The painted run must consume strictly more filament (the support material) —
/// a G-code-level proof that `SliceObject.support_paint` → the FFI's
/// `supported_facets` → libslic3r's manual-support path actually fires. If the
/// wiring were a no-op, both runs would report identical filament.
#[test]
fn painted_enforcer_grows_manual_tree_support() {
    ensure_ffi_init();

    // L-bracket: profile in XZ extruded along Y. Pillar x∈[0,4] z∈[0,20] sits on
    // the bed; arm x∈[0,20] z∈[16,20] cantilevers out, its underside (z=16,
    // x∈[4,20]) unsupported. 12 vertices (6-point profile × front/back), 20
    // triangles (2 caps × 4 + 6 side quads × 2).
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        // front (y=0): P0..P5
        0.0,0.0,0.0,  4.0,0.0,0.0,  4.0,0.0,16.0,  20.0,0.0,16.0,  20.0,0.0,20.0,  0.0,0.0,20.0,
        // back (y=10): P0'..P5'
        0.0,10.0,0.0, 4.0,10.0,0.0, 4.0,10.0,16.0, 20.0,10.0,16.0, 20.0,10.0,20.0, 0.0,10.0,20.0,
    ];
    #[rustfmt::skip]
    let indices: Vec<u32> = vec![
        // front cap (verts 0..5)
        0,1,2,  0,2,5,  2,3,4,  2,4,5,
        // back cap (verts 6..11)
        6,7,8,  6,8,11, 8,9,10, 8,10,11,
        // side walls: profile edges (a,b) → (a,b,b+6),(a,b+6,a+6)
        0,1,7,  0,7,6,     // edge 0-1 (pillar base underside/front-bottom)
        1,2,8,  1,8,7,     // edge 1-2 (pillar outer face)
        2,3,9,  2,9,8,     // edge 2-3 (ARM UNDERSIDE, z=16 — the overhang)
        3,4,10, 3,10,9,    // edge 3-4 (arm end)
        4,5,11, 4,11,10,   // edge 4-5 (top)
        5,0,6,  5,6,11,    // edge 5-0 (back/left face)
    ];

    // Enforcer-paint the arm underside: every triangle whose 3 vertices all sit
    // at z=16 (the downward overhang quad). "4" is the FacetsAnnotation leaf for
    // ENFORCER (state 1 << 2); "" leaves a facet unpainted. Position-based so
    // it's winding-independent.
    let support_hex: Vec<String> = indices
        .chunks_exact(3)
        .map(|t| {
            let z = |v: u32| verts[(v as usize) * 3 + 2];
            if t.iter().all(|&v| (z(v) - 16.0).abs() < 1e-3) {
                "4".to_string()
            } else {
                String::new()
            }
        })
        .collect();
    assert_eq!(
        support_hex.iter().filter(|s| !s.is_empty()).count(),
        2,
        "exactly the 2 arm-underside triangles should be enforcer-painted",
    );

    let verts = Arc::new(verts);
    let indices = Arc::new(indices);

    // Slice the bracket with the given support paint, returning total filament
    // grams. Placed near the bed centre; identity orientation.
    let slice_grams = |support_paint: Option<Arc<Vec<String>>>, tag: &str| -> f64 {
        let mut transform = [0.0f64; 16];
        transform[0] = 1.0;
        transform[5] = 1.0;
        transform[10] = 1.0;
        transform[15] = 1.0;
        transform[12] = 80.0; // x
        transform[13] = 80.0; // y

        let object = SliceObject {
            name: "bracket".into(),
            vertices: Arc::clone(&verts),
            indices: Arc::clone(&indices),
            paint: None,
            support_paint,
            transform,
            extruder: 1,
            overrides: vec![],
            group: None,
            group_overrides: vec![],
            modifiers: vec![],
        };

        let registry = JobRegistry::new();
        let (sink, events) = collecting_sink();
        let temp_dir =
            std::env::temp_dir().join(format!("n3o-manual-support-{tag}-{}", std::process::id()));
        let input = SliceJobInput {
            objects: vec![object],
            output_dir: temp_dir.display().to_string(),
            context: ContextJson {
                printer: bambi_printer(),
                plate: canonical_plate(),
                filaments: vec![canonical_filament()],
                active_slot: 0,
                user_overrides: vec![],
                project_overrides: vec![OverrideFileSpec {
                    label: "manual-support".into(),
                    content: "enable_support = \"1\"\nsupport_type = \"tree(manual)\"\n".into(),
                }],
                object_overrides: std::collections::HashMap::new(),
            },
            plate_ids: vec![1],
            printer_instance_id: "bambi".into(),
            material_layout: vec![],
            quality_profile: None,
            paint_filament_remap: None,
        };

        run_blocking(input, &registry, sink).expect("synchronous start");
        let events = events.lock().unwrap();
        if let Some(SliceEvent::JobFailed { error, .. }) = events
            .iter()
            .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
        {
            panic!("[{tag}] slice failed: {error:?}");
        }
        let summary = events
            .iter()
            .find_map(|e| match e {
                SliceEvent::PlateFinished { summary, .. } => Some(summary.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("[{tag}] no PlateFinished"));
        let _ = std::fs::remove_dir_all(&temp_dir);
        summary.filament_used_grams.values().sum()
    };

    let unpainted = slice_grams(None, "unpainted");
    let painted = slice_grams(Some(Arc::new(support_hex)), "painted");

    assert!(
        unpainted > 0.0 && painted > 0.0,
        "both slices must consume filament (unpainted={unpainted}, painted={painted})",
    );
    assert!(
        painted > unpainted * 1.05,
        "manual enforcer paint must add support material: painted={painted}g should exceed \
         unpainted={unpainted}g — the support-paint wiring is a no-op if they're equal",
    );
}

/// Leg 3 (deferred): copy-vs-vendor binding.
///
/// Once the in-app filament/process copy mechanic lands (tracked
/// MVP exclusion, settings-model.md §9), replace this with a real
/// assertion: copy a vendor filament, mutate the copy, slice with
/// the copy, verify the vendor source is unchanged and the slice
/// picked up the override. Until then this is `#[ignore]`d so the
/// expectation remains visible without breaking CI.
#[test]
#[ignore = "copy mechanic not yet implemented; tracked under PR-S-11 leg 3"]
fn copy_vs_vendor_binding_is_independent() {
    // Intentionally empty: leg-3 placeholder. See
    // settings-model.md §9 "In-app filament/process copy UX".
}
