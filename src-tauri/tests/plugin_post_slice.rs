//! Post-slice plugin hook, verified end-to-end against real G-code.
//!
//! Slices a real cube through the orchestrator with the bundled
//! example plugins active, then greps the output for the commands they
//! inject — green unit tests alone don't prove libslic3r's output
//! actually flowed through the hook.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::{ContextJson, OverrideFileSpec};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::gcode::{parse_str, to_string};
use n3o_slic3r_lib::core::plugin::{FilamentLoadout, PlateMeta, PluginHost, PostSliceHook};
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
    use n3o_slic3r_lib::core::gcode::Line;

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

    // With the example plugins active, their commands appear. Plugins
    // are opt-in, so enable the two this test asserts on.
    let host = Arc::new(Mutex::new(PluginHost::load(&[example_plugins_root()])));
    {
        let mut h = host.lock().unwrap();
        h.set_global_enabled("beep-at-layer", true);
        h.set_global_enabled("pause-at-layer", true);
    }
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

    let host = Arc::new(Mutex::new(PluginHost::load(&[plugins
        .path()
        .to_path_buf()])));
    host.lock().unwrap().set_global_enabled("force-bed", true);
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

/// Load a single bundled example plugin in isolation (copied into a
/// fresh temp root, so sibling examples don't also load).
fn host_for_example(name: &str) -> PluginHost {
    // The flagship platecycler ships in the bundled `plugins/` dir; the
    // other examples live under `examples/plugins/`. Resolve either.
    let root = workspace_root();
    let bundled = root.join("plugins").join(name);
    let src = if bundled.is_dir() {
        bundled
    } else {
        root.join("examples/plugins").join(name)
    };
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join(name);
    std::fs::create_dir_all(&dst).unwrap();
    for f in ["plugin.toml", "main.lua"] {
        std::fs::copy(src.join(f), dst.join(f)).unwrap();
    }
    // `load` reads the entry Lua into the runtime; the temp dir can drop.
    // Plugins are opt-in (off by default), so enable the one under test.
    let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
    host.set_global_enabled(name, true);
    host
}

fn a1_mini_plate() -> PlateMeta {
    PlateMeta {
        plate_id: 1,
        printer_model: "Bambu Lab A1 mini".into(),
        bed_type: Some("Textured PEI".into()),
        object_count: None,
    }
}

/// The platecycler plugin appends its eject macro once, and re-running
/// the hook over already-cycled G-code is a no-op (idempotent sentinel).
#[test]
fn platecycler_inserts_eject_macro_inside_executable_block_idempotently() {
    let mut host = host_for_example("platecycler");
    let hook = PostSliceHook {
        plate: a1_mini_plate(),
        filament: FilamentLoadout::default(),
    };
    // Mirror Bambu structure: runnable block ends at EXECUTABLE_BLOCK_END,
    // then a trailing config/footer the firmware ignores.
    let gcode = "; EXECUTABLE_BLOCK_START\nG1 X0 Y0 F1200\nM104 S0\nM18 X Y Z\n\
                 ; EXECUTABLE_BLOCK_END\n; filament used [g] = 1.0\n";

    let out1 = to_string(&host.dispatch(&hook, parse_str(gcode)));
    assert!(out1.contains("; n3o:platecycler"), "sentinel inserted");
    assert!(out1.contains("G0 Y186.5 F2000"), "eject macro inserted");
    // The macro must be INSIDE the executable block (before END), or the
    // firmware would ignore it and the plate would never eject.
    let sentinel = out1.find("; n3o:platecycler").unwrap();
    let end = out1.rfind("EXECUTABLE_BLOCK_END").unwrap();
    assert!(sentinel < end, "macro must sit before EXECUTABLE_BLOCK_END");

    let out2 = to_string(&host.dispatch(&hook, parse_str(&out1)));
    assert_eq!(
        out1.matches("; n3o:platecycler").count(),
        1,
        "exactly one eject sequence"
    );
    assert_eq!(out2, out1, "re-running must not double-insert");
}

/// The filament-summary example reads the `filament` binding and
/// prepends a per-slot header. Guards the example against bit-rot and
/// exercises the read-only binding end-to-end through a real host.
#[test]
fn filament_summary_example_prepends_loadout_header() {
    use n3o_slic3r_lib::core::plugin::SlotInfo;
    let mut host = host_for_example("filament-summary");
    let filament = FilamentLoadout {
        printer_model: "Bambu Lab A1 mini".into(),
        toolhead_count: 1,
        slots: vec![
            SlotInfo {
                index: 1,
                extruder: 0,
                slot: 0,
                feed: "ams",
                identity: Some("generic-pla".into()),
                base_type: Some("PLA".into()),
                color: Some("#ff8800".into()),
                vendor: Some("Generic".into()),
            },
            SlotInfo {
                index: 2,
                extruder: 0,
                slot: 1,
                feed: "ams",
                identity: None,
                base_type: None,
                color: None,
                vendor: None,
            },
        ],
    };
    let hook = PostSliceHook {
        plate: a1_mini_plate(),
        filament,
    };
    let gcode = "G1 X0 Y0 F1200\n";
    let out = to_string(&host.dispatch(&hook, parse_str(gcode)));
    assert!(out.starts_with("; n3o filament loadout for Bambu Lab A1 mini"));
    assert!(out.contains("; slot 1 (ams): generic-pla [PLA #ff8800 Generic]"));
    assert!(out.contains("; slot 2 (ams): <empty>"));
    // The original toolpath survives below the header.
    assert!(out.contains("G1 X0 Y0 F1200"));
}

/// The plugin's printer self-guard: it does nothing for a non-A1-mini
/// plate (printer_compatibility isn't host-enforced yet).
#[test]
fn platecycler_skips_non_a1_mini() {
    let mut host = host_for_example("platecycler");
    let hook = PostSliceHook {
        plate: PlateMeta {
            printer_model: "Snapmaker U1".into(),
            ..a1_mini_plate()
        },
        filament: FilamentLoadout::default(),
    };
    let gcode = "G1 X0 Y0 F1200\n";
    let out = to_string(&host.dispatch(&hook, parse_str(gcode)));
    assert_eq!(out, gcode, "no eject macro on a different printer");
}

/// Verify-via-G-code: a real A1 mini slice with the platecycler plugin
/// active carries the eject sequence at the tail.
#[test]
fn platecycler_eject_macro_in_real_slice() {
    ensure_ffi_init();
    let host = Arc::new(Mutex::new(host_for_example("platecycler")));
    let (input, registry, _out) = slice_input(vec![1]);
    let (sink, events) = collecting_sink();
    run_slice_job_blocking_with_plugins(input, &registry, sink, host).expect("plugin slice");
    let g = output_gcode(&events);

    assert!(g.contains("; n3o:platecycler"), "sentinel present");
    assert!(g.contains("G0 Y186.5 F2000"), "eject macro present");
    // It must land inside the executable block (before the END marker),
    // not past it where the firmware would ignore it.
    let sentinel = g.find("; n3o:platecycler").unwrap();
    let end = g
        .rfind("EXECUTABLE_BLOCK_END")
        .expect("real A1 mini slice has an executable-block end marker");
    assert!(
        sentinel < end,
        "eject macro must sit inside the executable block, before END"
    );
}

/// End-to-end activation gating: a `plugin.platecycler.enabled = false`
/// override in the job's tiers turns the plugin off for the slice — the
/// eject macro is absent even though the plugin is loaded and the
/// printer model matches.
#[test]
fn platecycler_disabled_by_activation_override_in_real_slice() {
    ensure_ffi_init();
    let host = Arc::new(Mutex::new(host_for_example("platecycler")));
    let (mut input, registry, _out) = slice_input(vec![1]);
    // Disable the plugin at the plate level (cascade project tier).
    input.context.project_overrides.push(OverrideFileSpec {
        label: "<test>".into(),
        content: "\"plugin.platecycler.enabled\" = false".into(),
    });
    let (sink, events) = collecting_sink();
    run_slice_job_blocking_with_plugins(input, &registry, sink, host).expect("plugin slice");
    let g = output_gcode(&events);
    assert!(
        !g.contains("; n3o:platecycler"),
        "a plugin deactivated via override must not append its macro"
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

    let host = Arc::new(Mutex::new(PluginHost::load(&[plugins
        .path()
        .to_path_buf()])));
    host.lock().unwrap().set_global_enabled("boom", true);
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

/// Phase 8 exit-criteria smoke (software chain): an example plugin is
/// discovered + loaded **off by default**, runs at post-slice once
/// enabled, and is suppressible per plate. See docs/phase-8-smoke.md.
#[test]
fn phase_8_exit_smoke() {
    ensure_ffi_init();
    const BEEP: &str = "M300 S440 P200";

    // (1) Discover + load the beep example into a fresh root.
    let src = workspace_root().join("examples/plugins/beep-at-layer");
    let tmp = tempfile::tempdir().unwrap();
    let dst = tmp.path().join("beep-at-layer");
    std::fs::create_dir_all(&dst).unwrap();
    for f in ["plugin.toml", "main.lua"] {
        std::fs::copy(src.join(f), dst.join(f)).unwrap();
    }
    let host = Arc::new(Mutex::new(PluginHost::load(&[tmp.path().to_path_buf()])));

    let slice_once = |host: Arc<Mutex<PluginHost>>, plate_off: bool| -> String {
        let (mut input, registry, _out) = slice_input(vec![1]);
        if plate_off {
            input.context.project_overrides.push(OverrideFileSpec {
                label: "<smoke>".into(),
                content: "\"plugin.beep-at-layer.enabled\" = false".into(),
            });
        }
        let (sink, events) = collecting_sink();
        run_slice_job_blocking_with_plugins(input, &registry, sink, host).expect("slice");
        output_gcode(&events)
    };

    // Off by default (opt-in) → no beep.
    assert!(
        !slice_once(host.clone(), false).contains(BEEP),
        "off by default → no beep"
    );

    // (2) Enabled globally → runs at post-slice.
    host.lock().unwrap().set_global_enabled("beep-at-layer", true);
    assert!(
        slice_once(host.clone(), false).contains(BEEP),
        "enabled → beep injected"
    );

    // (3) A per-plate off override suppresses it even with global on.
    assert!(
        !slice_once(host.clone(), true).contains(BEEP),
        "plate-level off override suppresses it"
    );
}

/// A broken plugin loads disabled with its error surfaced, and the
/// manual `plugin_reload` recovers it once the on-disk file is fixed.
/// (Exit criterion 4 — no FFI needed.)
#[test]
fn plugin_reload_recovers_a_broken_plugin() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fixme");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "name=\"fixme\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\n\
         hooks=[\"post_slice\"]\nenabled_by_default=true\n",
    )
    .unwrap();
    // A syntax error → the plugin loads in the errored state.
    std::fs::write(dir.join("main.lua"), "function on_post_slice(g, plate) return").unwrap();
    let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
    {
        let list = host.list();
        assert!(!list[0].enabled, "broken plugin loads disabled");
        assert!(list[0].last_error.is_some(), "with its error surfaced");
    }

    // Fix the file on disk, reload → recovers.
    std::fs::write(
        dir.join("main.lua"),
        r#"function on_post_slice(g, plate) g:append("; fixed") end"#,
    )
    .unwrap();
    host.reload("fixme").expect("reload a fixed plugin");
    let list = host.list();
    assert!(list[0].enabled, "reload of the fixed plugin recovers it");
    assert!(list[0].last_error.is_none(), "and clears the stale error");
}
