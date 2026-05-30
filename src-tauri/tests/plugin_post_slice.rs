//! Post-slice plugin hook, verified end-to-end against real G-code.
//!
//! Slices a real cube through the orchestrator with the bundled
//! example plugins active, then greps the output for the commands they
//! inject — green unit tests alone don't prove libslic3r's output
//! actually flowed through the hook.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::ContextJson;
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::plugin::PluginHost;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, run_slice_job_blocking_with_plugins, EventSink},
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

fn cube_stl() -> PathBuf {
    workspace_root().join("external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl")
}

fn example_plugins_root() -> PathBuf {
    workspace_root().join("examples/plugins")
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

/// A slice job over `plate_ids`, writing into a fresh unique temp dir
/// (returned so it outlives the slice — the orchestrator writes into it
/// and the test reads back).
fn slice_input(plate_ids: Vec<u32>) -> (SliceJobInput, JobRegistry, tempfile::TempDir) {
    let out = tempfile::tempdir().expect("temp dir");
    let input = SliceJobInput {
        model_path: cube_stl().display().to_string(),
        output_dir: out.path().display().to_string(),
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
    };
    (input, JobRegistry::new(), out)
}

fn plate_finished_count(events: &Arc<Mutex<Vec<SliceEvent>>>) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, SliceEvent::PlateFinished { .. }))
        .count()
}

fn output_gcode(events: &Arc<Mutex<Vec<SliceEvent>>>) -> String {
    let events = events.lock().unwrap();
    let path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(PathBuf::from(output_path)),
            _ => None,
        })
        .expect("a PlateFinished event with an output path");
    std::fs::read_to_string(&path).expect("read sliced gcode")
}

#[test]
fn post_slice_plugins_inject_into_real_gcode() {
    use n3o_slic3r_lib::core::gcode::{parse_str, to_string, Line};

    ensure_ffi_init();

    // Baseline: no plugins → libslic3r output has neither injection.
    let (input, registry, _out) = slice_input(vec![1]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink).expect("baseline slice");
    let baseline = output_gcode(&events);
    // Negative control for BOTH plugins' exact injected strings.
    assert!(
        !baseline.contains("M300 S440 P200"),
        "baseline shouldn't contain the beep"
    );
    assert!(
        !baseline.contains("M0 ; n3o pause-at-layer"),
        "baseline shouldn't contain the pause"
    );
    // Sanity: the example plugins target layer index 1, so the slice
    // must actually have >= 2 layers or the test proves nothing.
    let layer_count = parse_str(&baseline)
        .iter()
        .filter(|l| matches!(l, Line::LayerChange(_)))
        .count();
    assert!(
        layer_count >= 2,
        "fixture must slice to >= 2 layers (got {layer_count}); the example plugins target layer 1"
    );
    // Real-output round-trip: parse→serialize of libslic3r's own G-code
    // is byte-identical, so a no-op plugin leaves the file untouched
    // (apply_post_slice only rewrites when the bytes differ). This is
    // the contract the orchestrator's "skip write on no change" relies
    // on, tested against REAL output rather than a hand-written sample.
    assert_eq!(
        to_string(&parse_str(&baseline)),
        baseline,
        "parse→serialize of real libslic3r G-code must be byte-identical"
    );

    // With the example plugins active, their commands appear.
    let host = Arc::new(Mutex::new(PluginHost::load(&[example_plugins_root()])));
    let (input, registry, _out) = slice_input(vec![1]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking_with_plugins(input, &registry, sink, host).expect("plugin slice");
    let with_plugins = output_gcode(&events);

    assert!(
        with_plugins.contains("M300 S440 P200"),
        "beep-at-layer should have injected an M300"
    );
    assert!(
        with_plugins.contains("M0 ; n3o pause-at-layer"),
        "pause-at-layer should have injected its pause"
    );
}

/// A pre-slice plugin's edit to a resolved setting reaches libslic3r:
/// force the bed temperature to a distinctive value and confirm it
/// lands in the real G-code's bed-heat command.
#[test]
fn pre_slice_plugin_rewrites_bed_temp_in_real_gcode() {
    ensure_ffi_init();

    let (input, registry, _out) = slice_input(vec![1]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink).expect("baseline slice");
    let baseline = output_gcode(&events);
    assert!(
        !baseline.contains("M140 S42") && !baseline.contains("M190 S42"),
        "baseline bed temp shouldn't already be 42"
    );

    let plugins = tempfile::tempdir().unwrap();
    let dir = plugins.path().join("force-bed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "name=\"force-bed\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks=[\"pre_slice\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.lua"),
        r#"function on_pre_slice(s, ctx) s.bed_temp = "42" end"#,
    )
    .unwrap();

    let host = Arc::new(Mutex::new(PluginHost::load(&[plugins.path().to_path_buf()])));
    let (input, registry, _out) = slice_input(vec![1]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking_with_plugins(input, &registry, sink, host).expect("plugin slice");
    let with_plugin = output_gcode(&events);

    assert_ne!(
        with_plugin, baseline,
        "the pre-slice edit should change the output"
    );
    assert!(
        with_plugin.contains("M140 S42") || with_plugin.contains("M190 S42"),
        "bed_temp=42 should reach libslic3r as a 42C bed-heat command"
    );
}

/// A plugin that errors on one plate must not break the others: the job
/// completes every plate, the erroring plugin is isolated.
#[test]
fn erroring_plugin_does_not_break_a_multi_plate_job() {
    ensure_ffi_init();

    // A bundled-style plugin dir holding one always-erroring plugin.
    let plugins = tempfile::tempdir().unwrap();
    let dir = plugins.path().join("boom");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "name=\"boom\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks=[\"post_slice\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.lua"),
        r#"function on_post_slice(g, plate) error("boom") end"#,
    )
    .unwrap();

    let host = Arc::new(Mutex::new(PluginHost::load(&[plugins.path().to_path_buf()])));
    let (input, registry, _out) = slice_input(vec![1, 2]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking_with_plugins(input, &registry, sink, host)
        .expect("multi-plate slice should start");

    // Both plates finished despite the plugin erroring on the first.
    assert_eq!(
        plate_finished_count(&events),
        2,
        "an erroring plugin must not stop later plates"
    );
}
