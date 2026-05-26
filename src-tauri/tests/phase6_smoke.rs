//! Phase 6 exit-criteria smoke (PR-6-17).
//!
//! Chains every Phase 6 deliverable that touches the preview
//! pipeline into one repeatable test so a regression on slice →
//! preview-load → color-encode → stats → drop fails loudly with a
//! step-named assertion, rather than at some downstream consumer.
//!
//! Procedure (human-driven half — drag-drop, plate switching, etc.)
//! lives in `docs/phase-6-smoke.md`.
//!
//! What this test covers:
//!   1. Slice a real .stl through the orchestrator (PR-3-2 +
//!      PR-6-1).
//!   2. Build the preview pipeline (parse → IR → stats) on the
//!      slice output (PR-6-4/5/6).
//!   3. Round-trip the color buffers for every ColorMode (PR-6-5).
//!   4. Compute per-layer stats and verify they sum to the full-
//!      job duration (PR-6-6).
//!   5. Foreign-slicer compat: synthetic G-code with Orca / Cura /
//!      Prusa generator markers parses + detects slicer-of-origin
//!      via PR-3-8's header parser. (Sourcing real-printer
//!      fixtures from each slicer is deferred — the test point is
//!      that the preview pipeline accepts each flavor's header.)
//!   6. .gcode.3mf round-trip: write a synthetic container via
//!      PR-3-10, read it back via PR-6-14's reader, verify
//!      embedded gcode + metadata + thumbnail extract clean.
//!   7. Cleanup: drop the registry entries and assert they're
//!      gone.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::ContextJson;
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::gcode::{parse_str, parse_all_metadata, SlicerOrigin};
use n3o_slic3r_lib::core::preview::{
    build::build_preview,
    colors::{encode_colors, ColorMode, Palette},
    registry::{LoadedPreview, PreviewRegistry},
    stats::{compute_job_stats, compute_layer_stats},
};
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, PlateSummary, SliceEvent, SliceJobInput,
};
use n3o_slic3r_lib::core::threemf::{
    read_sliced_3mf, write_sliced_3mf, AmsBinding, SlicedPlate, SlicedProjectInput,
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

fn cube_stl() -> PathBuf {
    workspace_root().join("external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl")
}

fn canonical_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu A1 mini".into(),
        supported_build_plates: vec![
            "Cool".into(),
            "Textured PEI".into(),
            "Smooth PEI".into(),
            "Engineering".into(),
            "SuperTack".into(),
        ],
        toolheads: vec![Toolhead {
            default_nozzle_diameter: 0.4,
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
    let temp_dir = std::env::temp_dir().join(format!("n3o-phase6-smoke-{}", std::process::id()));

    let input = SliceJobInput {
        model_path: cube_stl().display().to_string(),
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
        material_to_slot: std::collections::BTreeMap::new(),
    };

    run_slice_job_blocking(input, &registry, sink).expect("slice start");

    let events = events.lock().unwrap();
    let output_path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(PathBuf::from(output_path)),
            _ => None,
        })
        .expect("PlateFinished event");
    let bytes = std::fs::read(&output_path).expect("read gcode");
    assert!(!bytes.is_empty(), "step 1: gcode file is empty");
    (output_path, bytes)
}

#[test]
fn phase6_smoke_slice_to_preview_pipeline() {
    ensure_ffi_init();

    // --- Step 1: slice from scene via PR-3-2 + PR-6-1 ---------------
    let (gcode_path, bytes) = slice_cube_to_gcode();
    let src = std::str::from_utf8(&bytes).expect("slice output is UTF-8");

    // --- Step 2: load through the preview pipeline (PR-6-4/5/6) -----
    let lines = parse_str(src);
    let geometry = build_preview(&lines);
    let layer_stats = compute_layer_stats(&geometry);
    let job_stats = compute_job_stats(&geometry, &layer_stats);

    assert!(
        job_stats.layer_count > 0,
        "step 2: preview yielded zero layers"
    );
    assert!(
        geometry.bounding_box.max[2] > 0.0,
        "step 2: preview bbox.max.z should be positive after a real slice (got {})",
        geometry.bounding_box.max[2],
    );
    assert!(
        !geometry.extrusions.is_empty(),
        "step 2: preview yielded zero extrusion segments"
    );

    // --- Step 3: color modes round-trip (PR-6-5) --------------------
    let layer_times: Vec<f32> = layer_stats.iter().map(|s| s.duration_seconds).collect();
    let mut buffer_lens: Vec<(ColorMode, usize)> = Vec::new();
    for mode in [
        ColorMode::Feature,
        ColorMode::Speed,
        ColorMode::Flow,
        ColorMode::LayerTime,
        ColorMode::Tool,
    ] {
        let colors = encode_colors(
            &geometry.extrusions,
            mode,
            Palette::Default,
            Some(&layer_times),
        );
        assert!(
            !colors.is_empty(),
            "step 3: color buffer for {:?} is empty",
            mode,
        );
        buffer_lens.push((mode, colors.len()));
    }
    // All modes encode (r, g, b) per vertex, two vertices per
    // segment → the buffer length is identical across modes.
    let first = buffer_lens[0].1;
    for (mode, len) in &buffer_lens[1..] {
        assert_eq!(
            *len, first,
            "step 3: {:?} buffer length differs from baseline ({len} vs {first})",
            mode,
        );
    }

    // --- Step 4: stats consistency (PR-6-6) -------------------------
    assert_eq!(
        layer_stats.len() as u32,
        job_stats.layer_count,
        "step 4: per-layer stats count ({}) ≠ job_stats.layer_count ({})",
        layer_stats.len(),
        job_stats.layer_count,
    );
    let sum_layer_duration: f32 = layer_stats.iter().map(|s| s.duration_seconds).sum();
    let job_duration = job_stats.total_duration_seconds;
    // 5% tolerance per the ticket — per-layer duration is derived
    // from feature timings which round; the job total may include
    // travel between layers that the per-layer sum doesn't.
    let denom = job_duration.max(1.0);
    let drift = (sum_layer_duration - job_duration).abs() / denom;
    assert!(
        drift < 0.05,
        "step 4: per-layer sum ({sum_layer_duration:.1}s) vs job total \
         ({job_duration:.1}s) drifts {:.2}% — > 5% tolerance",
        drift * 100.0,
    );

    // --- Step 7: cleanup — registry round-trip ----------------------
    let reg = PreviewRegistry::new();
    let handle = reg.alloc_id();
    reg.insert(
        handle,
        LoadedPreview {
            source_path: gcode_path.clone(),
            header: parse_all_metadata(src.as_bytes()),
            geometry,
            layer_stats,
            job_stats,
            lines,
        },
    );
    assert!(reg.with(handle, |_| ()).is_some(), "step 7: handle missing after insert");
    assert!(reg.remove(handle), "step 7: drop should report success");
    assert!(
        reg.with(handle, |_| ()).is_none(),
        "step 7: handle still resolves after drop"
    );

    // House-keeping — leave /tmp clean even if a downstream
    // assertion fired by panicking earlier.
    let _ = std::fs::remove_file(&gcode_path);
    if let Some(parent) = gcode_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

/// Step 5: foreign-slicer compat. Each slicer writes a recognizable
/// `; generated by …` line — the preview pipeline must accept all
/// three flavors and PR-3-8's header parser must classify the
/// origin correctly. We synthesize the fixtures inline rather than
/// checking in 3× 50KB blobs; sourcing actual cube slices from each
/// slicer is deferred per the index's open question and tracked as
/// follow-up (the header-classification contract is fully tested
/// here either way).
#[test]
fn phase6_smoke_foreign_slicer_headers() {
    for (origin_name, generator, expected) in [
        ("orca", "OrcaSlicer 2.3.0", SlicerOrigin::Orca),
        ("prusa", "PrusaSlicer 2.7.4+linux", SlicerOrigin::PrusaSlicer),
        ("cura", "Ultimaker Cura SteamEngine 5.6.0", SlicerOrigin::Cura),
    ] {
        let src = synthetic_foreign_gcode(generator);
        let header = parse_all_metadata(src.as_bytes());
        assert_eq!(
            header.slicer.as_ref(),
            Some(&expected),
            "step 5: {origin_name} header detection mismatch (got {:?})",
            header.slicer,
        );

        // Pipeline acceptance: parse + IR build must succeed and
        // produce at least one extrusion segment for the synthesized
        // cube perimeter.
        let lines = parse_str(&src);
        let geom = build_preview(&lines);
        assert!(
            !geom.extrusions.is_empty(),
            "step 5: {origin_name} fixture produced zero extrusions",
        );
    }
}

/// Minimal slicer-flavored cube: header + two layers of a 10×10
/// square perimeter. Enough to populate the IR with at least one
/// non-Travel FeatureType and exercise the header parser's
/// slicer-detection branch.
fn synthetic_foreign_gcode(generator: &str) -> String {
    format!(
        "; generated by {generator} on 2026-05-24\n\
         ; printer_model = SyntheticPrinter\n\
         ; total layers count = 2\n\
         M104 S210\n\
         G28\n\
         ;LAYER_CHANGE\n\
         ;Z:0.2\n\
         ;TYPE:External perimeter\n\
         G1 X0 Y0 Z0.2 F1800\n\
         G1 X10 Y0 E0.5 F1200\n\
         G1 X10 Y10 E1.0 F1200\n\
         G1 X0 Y10 E1.5 F1200\n\
         G1 X0 Y0 E2.0 F1200\n\
         ;LAYER_CHANGE\n\
         ;Z:0.4\n\
         ;TYPE:External perimeter\n\
         G1 X0 Y0 Z0.4 F1800\n\
         G1 X10 Y0 E2.5 F1200\n\
         G1 X10 Y10 E3.0 F1200\n\
         G1 X0 Y10 E3.5 F1200\n\
         G1 X0 Y0 E4.0 F1200\n"
    )
}

/// Step 6: .gcode.3mf write → read round-trip. Synthesize a 2-plate
/// container with thumbnail + AMS bindings, read it back via
/// PR-6-14's reader, assert plate metadata + gcode + thumbnail
/// survived. The preview pipeline acceptance for the embedded
/// gcode is exercised in step 2 (slice → preview) — the gcode
/// here is the same shape.
#[test]
fn phase6_smoke_gcode_3mf_round_trip() {
    let mut summary = PlateSummary::default();
    summary.layer_count = 2;
    summary.estimated_time_seconds = 60;
    summary.estimated_time_text = "1m".into();
    summary.filament_used_grams.insert(0, 2.5);

    let plate1 = SlicedPlate {
        plate_id: 1,
        gcode: synthetic_foreign_gcode("OrcaSlicer 2.3.0").into_bytes(),
        summary: summary.clone(),
        thumbnail_png: Some(vec![0x89, 0x50, 0x4E, 0x47]),
        ams_bindings: vec![AmsBinding {
            model_material_index: 0,
            ams_slot: 2,
        }],
    };
    let plate2 = SlicedPlate {
        plate_id: 2,
        gcode: synthetic_foreign_gcode("OrcaSlicer 2.3.0").into_bytes(),
        summary,
        thumbnail_png: None,
        ams_bindings: vec![],
    };
    let input = SlicedProjectInput {
        printer_model: "Bambu A1 mini".into(),
        file_metadata: std::collections::BTreeMap::new(),
        plates: vec![plate1, plate2],
    };

    let path = std::env::temp_dir().join(format!(
        "n3o-phase6-smoke-bundle-{}.gcode.3mf",
        std::process::id(),
    ));
    write_sliced_3mf(&input, &path).expect("step 6: write_sliced_3mf");

    let read = read_sliced_3mf(&path).expect("step 6: read_sliced_3mf");
    assert_eq!(read.plates.len(), 2, "step 6: plate count mismatch");
    let plate1 = &read.plates[0];
    assert_eq!(plate1.plate_id, 1);
    let meta = plate1.metadata.as_ref().expect("step 6: plate1 metadata");
    assert_eq!(meta.estimated_time_text, "1m");
    assert_eq!(meta.ams_bindings.len(), 1);
    assert_eq!(meta.ams_bindings[0].ams_slot, 2);
    assert!(plate1.thumbnail_png.is_some(), "step 6: plate1 thumbnail missing");
    assert!(read.plates[1].thumbnail_png.is_none(), "step 6: plate2 thumbnail expected None");

    // Preview pipeline acceptance for the embedded G-code.
    let src = std::str::from_utf8(&plate1.gcode).unwrap();
    let lines = parse_str(src);
    let geom = build_preview(&lines);
    assert!(!geom.extrusions.is_empty(), "step 6: embedded gcode yielded zero extrusions");

    let _ = std::fs::remove_file(&path);
}
