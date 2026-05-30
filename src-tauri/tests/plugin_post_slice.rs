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

fn slice_input(tag: &str) -> (SliceJobInput, JobRegistry) {
    let temp_dir = std::env::temp_dir().join(format!("n3o-post-slice-{tag}-{}", std::process::id()));
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
        material_layout: vec![],
    };
    (input, JobRegistry::new())
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
    ensure_ffi_init();

    // Baseline: no plugins → libslic3r output has neither injection.
    let (input, registry) = slice_input("baseline");
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink).expect("baseline slice");
    let baseline = output_gcode(&events);
    assert!(
        !baseline.contains("M300 S440 P200"),
        "baseline slice shouldn't contain the plugin's beep"
    );

    // With the example plugins active, their commands appear.
    let host = Arc::new(Mutex::new(PluginHost::load(&[example_plugins_root()])));
    let (input, registry) = slice_input("plugins");
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
