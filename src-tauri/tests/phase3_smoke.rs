#![cfg(feature = "test-fixtures")]
//! Phase 3 exit-criteria smoke.
//!
//! Chains every Phase 3 deliverable into one repeatable test so a
//! future regression that breaks any link — slice, parse, sliced-3MF
//! write — fails loudly with a step-named assertion rather than at the
//! leaf module's own unit test.
//!
//! The 50 MB / 3 s parser perf gate from PR-3-6 lives in
//! `gcode_parser_perf.rs` and runs as part of the same
//! `cargo test --workspace` step; we don't duplicate it here.

use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::ContextJson;
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::gcode::{parser, serializer};
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::input::SliceObject;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, SliceEvent, SliceJobInput,
};
use n3o_slic3r_lib::core::threemf::{fixture_input, write_sliced_3mf};
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


/// The cube STL loaded into a single buffer-load [`SliceObject`].
fn cube_objects() -> Vec<SliceObject> {
    let m = n3o_slic3r_lib::core::scene::loaders::load_mesh_from_path(&cube_stl())
        .expect("load cube STL");
    vec![SliceObject {
        name: "cube".into(),
        vertices: Arc::new(m.vertices),
        indices: Arc::new(m.indices),
        paint: m.paint_colors.map(Arc::new),
        support_paint: None,
        transform: [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ],
        extruder: 1,
        overrides: vec![],
        group: None,
        group_overrides: vec![],
        modifiers: vec![],
    }]
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

fn cube_stl() -> PathBuf {
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

fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<SliceEvent>>>) {
    let bucket: Arc<Mutex<Vec<SliceEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let bucket_for_cb = bucket.clone();
    let sink: EventSink = Box::new(move |event| {
        bucket_for_cb.lock().unwrap().push(event);
    });
    (sink, bucket)
}

fn slice_cube_to_gcode() -> (PathBuf, Vec<u8>) {
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    let temp_dir = std::env::temp_dir().join(format!("n3o-phase3-smoke-{}", std::process::id(),));

    let input = SliceJobInput {
        objects: cube_objects(),
        output_dir: temp_dir.display().to_string(),
        context: ContextJson {
            printer: canonical_printer(),
            plate: canonical_plate(),
            filaments: vec![canonical_filament()],
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids: vec![1],
        printer_instance_id: "bambi".into(),
        material_layout: vec![],
        quality_profile: None,
        paint_filament_remap: None,
    };

    let _job_id = run_blocking(input, &registry, sink).expect("slice start");

    let events = events.lock().unwrap();
    let (output_path, summary) = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished {
                output_path,
                summary,
                ..
            } => Some((PathBuf::from(output_path), summary.clone())),
            _ => None,
        })
        .expect("PlateFinished event with summary");

    assert!(
        summary.estimated_time_seconds > 0,
        "step 1: summary should carry a non-zero time estimate, got {:?}",
        summary,
    );
    assert!(
        summary.layer_count > 0,
        "step 1: summary should carry a non-zero layer count, got {}",
        summary.layer_count,
    );

    let bytes = std::fs::read(&output_path).expect("read output gcode");
    assert!(!bytes.is_empty(), "step 1: gcode file is empty");
    (output_path, bytes)
}

/// Steps 1-3 + step 6 chained: slice → parse → byte-equal serialize
/// → bundle the slice output into a `.gcode.3mf`. Steps in the same
/// test so the failure points at the broken link directly; cleanup
/// happens at the end regardless of which assertion blew up.
#[test]
fn phase3_smoke_slice_parse_roundtrip_bundle() {
    ensure_ffi_init();

    // --- Step 1: slice the cube end-to-end through PR-3-2 -----------
    let (gcode_path, original_bytes) = slice_cube_to_gcode();

    // --- Step 2: parse via PR-3-6 with zero ParseError --------------
    let mut errors: Vec<parser::ParseError> = Vec::new();
    let mut lines = Vec::new();
    for r in parser::parse_lines(&original_bytes[..]) {
        match r {
            Ok(line) => lines.push(line),
            Err(e) => errors.push(e),
        }
    }
    assert!(
        errors.is_empty(),
        "step 2: parser raised {} errors on libslic3r output: first = {}",
        errors.len(),
        errors[0],
    );
    assert!(
        !lines.is_empty(),
        "step 2: parser yielded zero lines on non-empty gcode"
    );

    // --- Step 3: serialize via PR-3-7, byte-equal with the input ----
    let mut reserialized = Vec::with_capacity(original_bytes.len());
    serializer::write_lines(&lines, &mut reserialized).expect("write_lines");
    assert_eq!(
        reserialized.len(),
        original_bytes.len(),
        "step 3: re-serialized length differs (orig {} vs roundtrip {})",
        original_bytes.len(),
        reserialized.len(),
    );
    if reserialized != original_bytes {
        // Pinpoint the first diverging byte so a future regression
        // shows up as "line N differs" rather than "byte vec differ".
        let mismatch = original_bytes
            .iter()
            .zip(reserialized.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let start = mismatch.saturating_sub(40);
        let end_a = (mismatch + 40).min(original_bytes.len());
        let end_b = (mismatch + 40).min(reserialized.len());
        panic!(
            "step 3: parser → serializer not byte-equivalent\n\
             first diff @ byte {mismatch}\n\
             orig: {:?}\nback: {:?}",
            String::from_utf8_lossy(&original_bytes[start..end_a]),
            String::from_utf8_lossy(&reserialized[start..end_b]),
        );
    }

    // --- Step 6: sliced .gcode.3mf via PR-3-10 ----------------------
    // Wrap the real slice output (not a synthetic fixture) so the
    // smoke proves the whole stack composes — orchestrator output
    // feeds the bundler unchanged.
    let bundle_path = std::env::temp_dir().join(format!(
        "n3o-phase3-smoke-bundle-{}.gcode.3mf",
        std::process::id(),
    ));
    let input = fixture_input(1, original_bytes.clone());
    write_sliced_3mf(&input, &bundle_path).expect("step 6: write_sliced_3mf");

    let extracted = {
        let f = std::fs::File::open(&bundle_path).expect("open bundle");
        let mut zip = zip::ZipArchive::new(f).expect("open zip");
        let mut entry = zip
            .by_name("Metadata/plate_1.gcode")
            .expect("plate_1.gcode entry");
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).expect("read entry");
        buf
    };
    assert_eq!(
        extracted, original_bytes,
        "step 6: bundled gcode does not match the slice output byte-for-byte",
    );

    let _ = std::fs::remove_file(&gcode_path);
    if let Some(parent) = gcode_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    let _ = std::fs::remove_file(&bundle_path);
}

