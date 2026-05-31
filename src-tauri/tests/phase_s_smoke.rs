//! PR-S exit-criteria smoke (PR-S-11).
//!
//! Locks the multi-instance + multi-filament work from PR-S-1
//! through PR-S-10 into CI. If the cascade composer regresses
//! filament fan-out, per-OptType separator dispatch, or
//! `flush_volumes_matrix` sizing, this fails loudly before a real
//! slice ever runs.
//!
//! Three legs from docs/settings-model.md §11.4:
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
//!
//! Procedure for the human-driven half lives in
//! `docs/phase-s-smoke.md`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
// Mutex is for the per-test event-collecting sinks only; the FFI
// slice itself serializes inside `slic3r_ffi::slice`'s process-wide
// SLICE_LOCK, so no test-side lock needed.

use n3o_slic3r_lib::core::cascade::commands::{ContextJson, OverrideFileSpec};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, PlateSummary, SliceEvent, SliceJobInput,
};
use slic3r_ffi::init as ffi_init;

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

    let input = SliceJobInput {
        model_path: fourcolor_3mf().display().to_string(),
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
    };

    run_slice_job_blocking(input, &registry, sink)
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

/// Material→slot binding routes a single-material print to the
/// bound toolhead on a toolchanger.
///
/// Loads OrcaCube_v2 into a Project on snappy, binds model material
/// 1 to T1's slot (extruder=1, slot=0), runs through
/// `build_slice_input` (the production path), and verifies the
/// emitted gcode contains `T1` toolchange markers.
///
/// Toolchangers use the legacy slot-fanned cascade and the
/// per-object remap: material 1 → bound flat slot index 1 →
/// libslic3r filament index 2. The temp `.3mf` carries the
/// remapped value (`extruder_id = 2`) so libslic3r's gcode
/// template emits the right `T<n>` for the firmware to select
/// the right toolhead.
#[test]
fn snappy_binding_routes_single_material_to_bound_toolhead() {
    use n3o_slic3r_lib::core::printer::SlotRef;
    use n3o_slic3r_lib::core::project::{PlateId, Project};
    use n3o_slic3r_lib::core::scene::state::NewSceneObject;
    use n3o_slic3r_lib::core::slice::input::build_slice_input;
    use n3o_slic3r_lib::core::threemf::load_3mf;

    ensure_ffi_init();

    let cube_path = workspace_root().join("assets/calibration/OrcaCube_v2.3mf");

    // Build a Project the way the UI's scene_load_3mf does: load the
    // 3mf, register every mesh + object on the active plate.
    let project_3mf = load_3mf(&cube_path).expect("load OrcaCube");
    let mut project = Project::default();
    project.plates[0].printer_instance_id = Some("snappy".into());
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
            parent: None,
            group_id: obj.group_id,
        });
    }

    // Force the M1 → T1 binding (override the auto-bound default).
    project.plates[0].material_to_slot.insert(
        1,
        SlotRef {
            extruder: 1,
            slot: 0,
        },
    );

    let temp_dir = std::env::temp_dir().join(format!("n3o-snappy-binding-{}", std::process::id(),));
    let (input, temp_3mf) = build_slice_input(&project, PlateId(1), temp_dir.display().to_string())
        .expect("build_slice_input");

    // The cube's recorded extruder in the temp .3mf must already be
    // 2 (libslic3r 1-based filament index = bound flat slot 1 + 1
    // on snappy). Pin it here so a regression is caught even if the
    // slice itself fails for unrelated reasons.
    let reloaded = load_3mf(&temp_3mf).expect("reload temp 3mf");
    assert!(
        reloaded.objects.iter().any(|o| o.extruder_id == Some(2)),
        "expected ≥1 object with remapped extruder_id=2, got {:?}",
        reloaded
            .objects
            .iter()
            .map(|o| o.extruder_id)
            .collect::<Vec<_>>(),
    );

    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink).expect("synchronous start");

    let events = events.lock().unwrap();
    if let Some(SliceEvent::JobFailed { error, .. }) = events
        .iter()
        .find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        panic!("slice failed: {error:?}");
    }
    let gcode_path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(PathBuf::from(output_path)),
            _ => None,
        })
        .expect("PlateFinished");

    let gcode = std::fs::read_to_string(&gcode_path).expect("read gcode");
    let t1_lines = gcode.lines().filter(|l| l.trim() == "T1").count();
    let t0_bare = gcode.lines().filter(|l| l.trim() == "T0").count();
    assert!(
        t1_lines >= 1,
        "expected ≥1 bare `T1` toolchange (M1 bound to T1), got {t1_lines} (T0 count: {t0_bare})",
    );

    let _ = std::fs::remove_dir_all(temp_dir);
    let _ = std::fs::remove_file(temp_3mf);
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
    use n3o_slic3r_lib::core::slice::input::build_slice_input;
    use n3o_slic3r_lib::core::threemf::load_3mf;

    ensure_ffi_init();

    let fixture = workspace_root().join("src-tauri/tests/fixtures/3mf/cube-halves-2mat.3mf");
    let project_3mf = load_3mf(&fixture).expect("load cube-halves fixture");

    let mut project = Project::default();
    project.plates[0].printer_instance_id = Some("bambi".into());
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
            parent: None,
            group_id: obj.group_id,
        });
    }

    let temp_dir = std::env::temp_dir().join(format!("n3o-cube-halves-{}", std::process::id(),));
    let (input, temp_3mf) = build_slice_input(&project, PlateId(1), temp_dir.display().to_string())
        .expect("build_slice_input");

    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink).expect("synchronous start");

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
    let _ = std::fs::remove_file(temp_3mf);
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
