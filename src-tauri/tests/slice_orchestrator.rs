//! Slice orchestrator integration test (PR-3-2; reshaped by PR-S-5c).
//!
//! Drives the full chain end-to-end without spinning up Tauri:
//! construct a `SliceJobInput` with `printer_instance_id`, call
//! `run_slice_job_blocking`, assert the emitted `SliceEvent` stream
//! and the produced G-code file. The orchestrator composes the
//! cascade from the PR-S-4 per-bucket vendor fragments at slice time.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::ContextJson;
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, SliceEvent, SliceJobInput,
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

fn test_stl() -> PathBuf {
    workspace_root().join("external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl")
}

fn canonical_printer() -> PrinterProfile {
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

/// Sink that captures every emitted event into a shared Vec.
/// Returns the boxed sink + a handle for assertions.
fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<SliceEvent>>>) {
    let bucket: Arc<Mutex<Vec<SliceEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let bucket_for_cb = bucket.clone();
    let sink: EventSink = Box::new(move |event| {
        bucket_for_cb.lock().unwrap().push(event);
    });
    (sink, bucket)
}

fn bambi_input(model_path: String, output_dir: String, plate_ids: Vec<u32>) -> SliceJobInput {
    SliceJobInput {
        model_path,
        output_dir,
        context: ContextJson {
            printer: canonical_printer(),
            plate: canonical_plate(),
            filaments: vec![canonical_filament()],
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids,
        printer_instance_id: "bambi".into(),
        material_layout: vec![],
    }
}

fn snappy_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Snapmaker U1".into(),
        supported_build_plates: vec!["Textured PEI Plate".into()],
        toolheads: (0..4)
            .map(|_i| Toolhead {
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

fn snappy_input(model_path: String, output_dir: String, plate_ids: Vec<u32>) -> SliceJobInput {
    // 4 extruders × 1 slot — flat ContextJson.filaments is per-slot,
    // so populate four canonical PLAs even though the composer pulls
    // the real filament identity off the bound PrinterInstance.
    SliceJobInput {
        model_path,
        output_dir,
        context: ContextJson {
            printer: snappy_printer(),
            plate: canonical_plate(),
            filaments: (0..4).map(|_| canonical_filament()).collect(),
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids,
        printer_instance_id: "snappy".into(),
        material_layout: vec![],
    }
}

#[test]
fn bambi_slice_emits_started_progress_finished_with_summary() {
    ensure_ffi_init();
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();

    let temp_dir =
        std::env::temp_dir().join(format!("n3o-slice-orch-test-{}", std::process::id(),));

    let input = bambi_input(
        test_stl().display().to_string(),
        temp_dir.display().to_string(),
        vec![1],
    );

    let job_id = run_slice_job_blocking(input, &registry, sink).expect("start");
    assert_eq!(job_id.0, 1);

    let events = events.lock().unwrap();
    // Expected sequence (modulo many PlateProgress in the middle):
    // PlateStarted → PlateProgress×N → PlateFinished → JobFinished.
    assert!(
        matches!(
            events.first(),
            Some(SliceEvent::PlateStarted { plate_id: 1, .. })
        ),
        "first event should be PlateStarted, got {:?}",
        events.first(),
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SliceEvent::PlateProgress { .. })),
        "expected at least one PlateProgress event",
    );
    let finished = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished {
                plate_id,
                output_path,
                summary,
                ..
            } => Some((*plate_id, output_path.clone(), summary.clone())),
            _ => None,
        })
        .expect("expected PlateFinished");
    assert_eq!(finished.0, 1);
    // Output file exists.
    assert!(
        std::path::Path::new(&finished.1).exists(),
        "expected gcode at {}",
        finished.1,
    );
    // Summary picked up the time estimate from the libslic3r header
    // (real slice runs through `; estimated printing time = …`).
    assert!(
        finished.2.estimated_time_seconds > 0,
        "expected a non-zero time estimate; got summary {:?}",
        finished.2,
    );
    assert!(
        finished.2.layer_count > 0,
        "expected a non-zero layer count; got {}",
        finished.2.layer_count,
    );
    // Job-finished is the terminal event.
    assert!(
        matches!(events.last(), Some(SliceEvent::JobFinished { .. })),
        "last event should be JobFinished, got {:?}",
        events.last(),
    );

    // Clean up the temp dir.
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn snappy_slice_emits_started_progress_finished_with_summary() {
    // Sibling of the Bambi slice smoke. Exercises the 4-extruder
    // toolchanger path through the composer: each extruder loads its
    // own nozzle.toml, the cascade vector-assembles per-extruder
    // scalars into length-4 strings, and the Textured PEI bed
    // fragment is the only one Snapmaker authored. Also exercises
    // the synthesized `filament_map` topology — without it
    // `GCodeProcessor::update_slice_warnings` segfaults on the
    // default length-1 filament map (caught the first time this
    // test was written).
    ensure_ffi_init();
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();

    let temp_dir =
        std::env::temp_dir().join(format!("n3o-slice-orch-snappy-{}", std::process::id(),));

    let input = snappy_input(
        test_stl().display().to_string(),
        temp_dir.display().to_string(),
        vec![1],
    );

    let job_id = run_slice_job_blocking(input, &registry, sink).expect("start");
    assert_eq!(job_id.0, 1);

    let events = events.lock().unwrap();
    assert!(
        matches!(
            events.first(),
            Some(SliceEvent::PlateStarted { plate_id: 1, .. })
        ),
        "first event should be PlateStarted, got {:?}",
        events.first(),
    );
    let finished = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished {
                plate_id,
                output_path,
                summary,
                ..
            } => Some((*plate_id, output_path.clone(), summary.clone())),
            _ => None,
        })
        .expect("expected PlateFinished");
    assert_eq!(finished.0, 1);
    assert!(std::path::Path::new(&finished.1).exists());
    assert!(finished.2.estimated_time_seconds > 0);
    assert!(finished.2.layer_count > 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// Composer-only check for the Snappy path: build the cascade for the
/// snappy PrinterInstance and confirm it resolves without errors. This
/// is the most we can verify end-to-end while the real klipper slice
/// (above) is blocked on upstream.
#[test]
fn snappy_orchestrator_compose_succeeds() {
    use n3o_slic3r_lib::core::printer::lookup_instance;
    use n3o_slic3r_lib::core::profile_library::compose_cascade;
    use std::collections::BTreeMap;
    // FFI init is required so the schema cache is populated; the composer
    // dispatches separator choice on OptType (`;` for coStrings).
    ensure_ffi_init();
    let instance = lookup_instance("snappy").expect("snappy in instance library");
    let cascade =
        compose_cascade(&instance, &[], &BTreeMap::new()).expect("snappy cascade composes");
    assert!(!cascade.rules.is_empty(), "snappy cascade is empty");
    // Spot-check a per-extruder vector landed length-4 (one entry per
    // U1 toolhead): the composer assembles `nozzle_diameter` from each
    // extruder's installed_nozzle SKU.
    let set = cascade
        .rules
        .iter()
        .find_map(|r| r.set.get("nozzle_diameter"))
        .expect("nozzle_diameter assembled into the cascade");
    let parts: Vec<&str> = set.split(',').collect();
    assert_eq!(
        parts.len(),
        4,
        "expected 4-element nozzle_diameter vector, got {set:?}"
    );

    // And per-slot filament vectors fan out to length-4 (one per AMS/
    // toolchanger slot). `filament_diameter` is a coFloats key present
    // in every filament fragment.
    let diam = cascade
        .rules
        .iter()
        .find_map(|r| r.set.get("filament_diameter"))
        .expect("filament_diameter assembled into the cascade");
    let diam_parts: Vec<&str> = diam.split(',').collect();
    assert_eq!(
        diam_parts.len(),
        4,
        "expected 4-element filament_diameter vector, got {diam:?}"
    );

    // coStrings keys use ';' as the separator (libslic3r's
    // ConfigOptionStrings convention). filament_type ("PLA") doesn't
    // need quoting; verify both the separator choice and length-4.
    let ftype = cascade
        .rules
        .iter()
        .find_map(|r| r.set.get("filament_type"))
        .expect("filament_type assembled into the cascade");
    let ftype_parts: Vec<&str> = ftype.split(';').collect();
    assert_eq!(
        ftype_parts.len(),
        4,
        "expected 4-element ';'-joined filament_type, got {ftype:?}"
    );
    assert!(
        !ftype.contains(','),
        "filament_type must not use ',' (libslic3r treats it as data inside a string), got {ftype:?}"
    );
}

#[test]
fn empty_plate_list_errors_synchronously() {
    ensure_ffi_init();
    let registry = JobRegistry::new();
    let (sink, _events) = collecting_sink();

    let input = bambi_input(
        test_stl().display().to_string(),
        std::env::temp_dir().display().to_string(),
        vec![],
    );

    let err = run_slice_job_blocking(input, &registry, sink).expect_err("ok");
    use n3o_slic3r_lib::core::slice::SliceStartError;
    assert!(
        matches!(err, SliceStartError::NoPlatesRequested),
        "got {err:?}"
    );
}

#[test]
fn unknown_printer_instance_errors() {
    ensure_ffi_init();
    let registry = JobRegistry::new();
    let (sink, _events) = collecting_sink();

    let mut input = bambi_input(
        test_stl().display().to_string(),
        std::env::temp_dir().display().to_string(),
        vec![1],
    );
    input.printer_instance_id = "ghost-printer".into();

    let err = run_slice_job_blocking(input, &registry, sink).expect_err("ok");
    use n3o_slic3r_lib::core::slice::SliceStartError;
    assert!(
        matches!(err, SliceStartError::PrinterInstanceCompose(_)),
        "got {err:?}",
    );
}
