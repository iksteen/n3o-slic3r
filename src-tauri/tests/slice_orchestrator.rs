#![cfg(feature = "test-fixtures")]
//! Slice orchestrator integration test.
//!
//! Drives the full chain end-to-end without spinning up Tauri:
//! construct a `SliceJobInput` with `printer_instance_id`, call
//! `run_slice_job_blocking`, assert the emitted `SliceEvent` stream
//! and the produced G-code file. The orchestrator composes the
//! cascade from the PR-S-4 per-bucket vendor fragments at slice time.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::{ContextJson, OverrideFileSpec};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::input::SliceObject;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, SliceEvent, SliceJobInput,
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


/// Column-major identity object→world transform.
const IDENTITY16: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// The 20mm box STL loaded into a single buffer-load [`SliceObject`].
fn stl_objects() -> Vec<SliceObject> {
    let m = n3o_slic3r_lib::core::scene::loaders::load_mesh_from_path(&test_stl())
        .expect("load test STL");
    vec![SliceObject {
        name: "20mmbox".into(),
        vertices: Arc::new(m.vertices),
        indices: Arc::new(m.indices),
        paint: m.paint_colors.map(Arc::new),
        support_paint: None,
        transform: IDENTITY16,
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

fn bambi_input(objects: Vec<SliceObject>, output_dir: String, plate_ids: Vec<u32>) -> SliceJobInput {
    SliceJobInput {
        objects,
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
        quality_profile: None,
        paint_filament_remap: None,
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

fn snappy_input(objects: Vec<SliceObject>, output_dir: String, plate_ids: Vec<u32>) -> SliceJobInput {
    // 4 extruders × 1 slot — flat ContextJson.filaments is per-slot,
    // so populate four canonical PLAs even though the composer pulls
    // the real filament identity off the bound PrinterInstance.
    SliceJobInput {
        objects,
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
        quality_profile: None,
        paint_filament_remap: None,
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
        stl_objects(),
        temp_dir.display().to_string(),
        vec![1],
    );

    let job_id = run_blocking(input, &registry, sink).expect("start");
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

/// Slice a plate and return the emitted G-code text.
fn slice_to_gcode(label: &str, input: SliceJobInput) -> String {
    let instance = n3o_slic3r_lib::core::printer::lookup_instance(&input.printer_instance_id)
        .expect("test-fixtures: bound instance resolves");
    slice_to_gcode_with(label, input, &instance)
}

/// [`slice_to_gcode`] against a caller-supplied instance — for tests that
/// need slot bindings the bundled fixture doesn't carry.
fn slice_to_gcode_with(
    label: &str,
    input: SliceJobInput,
    instance: &n3o_slic3r_lib::core::printer::PrinterInstance,
) -> String {
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, instance, &registry, sink)
        .unwrap_or_else(|e| panic!("[{label}] start: {e:?}"));
    let path = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(output_path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("[{label}] no PlateFinished event"));
    let gcode =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("[{label}] read {path}: {e}"));
    let _ = std::fs::remove_dir_all(std::path::Path::new(&path).parent().unwrap());
    gcode
}

/// First element of a comma-joined config-trailer scalar, e.g.
/// `; textured_plate_temp = 65,65,65` → `"65"`.
fn config_value<'a>(gcode: &'a str, key: &str) -> &'a str {
    config_vector(gcode, key).split(',').next().unwrap().trim()
}

/// The whole comma-joined config-trailer value, e.g.
/// `; nozzle_temperature = 220,255` → `"220,255"`.
fn config_vector<'a>(gcode: &'a str, key: &str) -> &'a str {
    let needle = format!("; {key} = ");
    gcode
        .lines()
        .find_map(|l| l.strip_prefix(&needle))
        .unwrap_or_else(|| panic!("config trailer missing `{key}`"))
        .trim()
}

/// PR-9-1 (Phase 9) — slice-path correctness gate, at the G-code
/// boundary. Proves the *resolved cascade* is what libslic3r slices
/// from, not the engine's defaults or any embedded config (the input
/// is a raw STL — there is no embedded config to leak).
///
/// For each MVP printer, with the active plate `Textured PEI Plate`:
///   - `curr_bed_type` in the trailer is the context's plate type
///     (cascade context → adapter), and
///   - the engine's actual bed-heat commands (`M140`/`M190`) in the
///     body carry the cascade-resolved `textured_plate_temp` — i.e. the
///     resolved value reached the engine and was baked into output, and
///   - that temperature is a real heated value (> 40 °C), guarding the
///     U1 cold-bed class of bug (`textured_plate_temp = 0` leaking when a
///     per-printer rule fails to fire).
///
/// The two printers resolve to *different* bed temps from their own
/// fragments (A1 mini + bambu-pla → 65; U1 + generic-pla → 60), which a
/// shared engine default could not produce. The U1 + snapmaker-pla rule
/// that lowers this to 55 is guarded at the compose layer by
/// `composer::tests::u1_filament_fragment_printer_rule_fires_at_compose_time`;
/// the bundled `snappy` fixture binds generic-pla, so 60 is correct here.
#[test]
fn resolved_bed_temp_reaches_the_engine_for_both_printers() {
    ensure_ffi_init();
    let stl = stl_objects();
    let td = |name: &str| {
        std::env::temp_dir()
            .join(format!("n3o-9-1-{name}-{}", std::process::id()))
            .display()
            .to_string()
    };

    let mut resolved = vec![];
    for (label, mk) in [
        (
            "bambi",
            bambi_input as fn(Vec<SliceObject>, String, Vec<u32>) -> SliceJobInput,
        ),
        (
            "snappy",
            snappy_input as fn(Vec<SliceObject>, String, Vec<u32>) -> SliceJobInput,
        ),
    ] {
        let gcode = slice_to_gcode(label, mk(stl.clone(), td(label), vec![1]));

        // Context plate type reached the config (cascade → adapter).
        assert_eq!(
            config_value(&gcode, "curr_bed_type"),
            "Textured PEI Plate",
            "[{label}] curr_bed_type should be the context's plate type",
        );

        // The active plate's resolved bed temp is a real heated value,
        // not 0 (the cold-bed bug) and not unset.
        let bed = config_value(&gcode, "textured_plate_temp");
        let bed_c: u32 = bed
            .parse()
            .unwrap_or_else(|_| panic!("[{label}] bed temp `{bed}`"));
        assert!(
            bed_c > 40,
            "[{label}] resolved textured_plate_temp {bed_c} looks like a cold-bed leak"
        );

        // The engine baked that resolved value into the body's bed-heat
        // commands — proof the cascade value reached libslic3r, not just
        // the trailer echo.
        assert!(
            gcode.contains(&format!("M140 S{bed}")),
            "[{label}] expected `M140 S{bed}` (resolved bed temp) in the G-code body",
        );
        assert!(
            gcode.contains(&format!("M190 S{bed}")),
            "[{label}] expected `M190 S{bed}` (resolved bed temp) in the G-code body",
        );
        resolved.push((label, bed_c));
    }

    // The two printers resolve to different temps from their own
    // fragments — a shared engine default could not.
    assert_ne!(
        resolved[0].1, resolved[1].1,
        "per-printer cascade differentiation: {resolved:?} should differ",
    );
}

/// C-1 — the project-wide *user* override tier (`Project.user_overrides`)
/// must reach the engine. Before C-1 the slice path dropped it entirely
/// (only the per-plate project tier was folded), so a user override never
/// sliced. This sets `bed_temp` in the user tier to a distinctive value and
/// proves it both wins over the bambi fragment default (65) and is baked
/// into the body's bed-heat command — i.e. the second-phase tier resolution
/// runs at slice time.
#[test]
fn user_tier_override_reaches_the_engine() {
    ensure_ffi_init();
    let stl = stl_objects();
    let out = std::env::temp_dir()
        .join(format!("n3o-c1-user-tier-{}", std::process::id()))
        .display()
        .to_string();

    let mut input = bambi_input(stl, out, vec![1]);
    // `bed_temp` is the logical key the adapter broadcasts to every
    // plate-type temp; 53 is distinct from the bambu-pla fragment's 65.
    input.context.user_overrides = vec![OverrideFileSpec {
        label: "user-overrides.toml".into(),
        content: "bed_temp = \"53\"\n".into(),
    }];

    let gcode = slice_to_gcode("user-tier", input);
    assert_eq!(
        config_value(&gcode, "textured_plate_temp"),
        "53",
        "user-tier bed_temp override should win over the fragment default",
    );
    assert!(
        gcode.contains("M140 S53"),
        "user-tier override should reach the engine's bed-heat command",
    );
}

/// A grouped member's *object-scope* override (`enable_support`) must reach
/// the engine. Grouped objects collapse into one multi-volume ModelObject:
/// the group's own overrides ride `add_group` onto the ModelObject config,
/// and a *member*-stored object-scope key (legacy projects) is promoted off
/// the volume config by the FFI — libslic3r only reads region-scope keys
/// there. Before either path existed the override was silently ignored and
/// a floating group sliced with no support at all.
#[test]
fn grouped_member_object_scope_override_reaches_the_engine() {
    ensure_ffi_init();
    let group = Some(n3o_slic3r_lib::core::scene::state::GroupId::fresh());
    // One member on the plate (keeps the first layers non-empty, which
    // libslic3r validates), one floated 10mm up — the floating twin's flat
    // bottom is a full overhang: no support without the override,
    // unmistakable support columns with it.
    let floating = |member_overrides: Vec<(String, String)>,
                    group_overrides: Vec<(String, String)>| {
        let mut objs = stl_objects();
        objs[0].overrides = member_overrides;
        objs[0].group = group;
        objs[0].group_overrides = group_overrides;
        let mut twin = objs[0].clone();
        twin.name = "20mmbox-twin".into();
        twin.transform[12] = 30.0;
        twin.transform[14] = 10.0;
        twin.overrides = vec![];
        objs.push(twin);
        objs
    };
    let td = |name: &str| {
        std::env::temp_dir()
            .join(format!("n3o-grp-ovr-{name}-{}", std::process::id()))
            .display()
            .to_string()
    };
    // BBL-flavor gcode tags extrusion roles as `; FEATURE:`, other flavors
    // as `;TYPE:` — match either.
    let support_markers = |gcode: &str| {
        gcode
            .lines()
            .filter(|l| {
                (l.starts_with("; FEATURE:") || l.starts_with(";TYPE:"))
                    && l.to_lowercase().contains("support")
            })
            .count()
    };

    let support = vec![("enable_support".to_string(), "1".to_string())];

    let baseline = slice_to_gcode(
        "grp-baseline",
        bambi_input(floating(vec![], vec![]), td("off"), vec![1]),
    );
    assert_eq!(
        support_markers(&baseline),
        0,
        "baseline floating group should slice without support",
    );

    let group_override = slice_to_gcode(
        "grp-override",
        bambi_input(floating(vec![], support.clone()), td("grp"), vec![1]),
    );
    assert!(
        support_markers(&group_override) > 0,
        "the group's enable_support override should produce support extrusions",
    );

    let member_override = slice_to_gcode(
        "grp-member-legacy",
        bambi_input(floating(support, vec![]), td("member"), vec![1]),
    );
    assert!(
        support_markers(&member_override) > 0,
        "a member-stored enable_support override (legacy project) should still produce support",
    );
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
        stl_objects(),
        temp_dir.display().to_string(),
        vec![1],
    );

    let job_id = run_blocking(input, &registry, sink).expect("start");
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
    // FFI init is required so the schema cache is populated; the composer
    // dispatches separator choice on OptType (`;` for coStrings).
    ensure_ffi_init();
    let instance = lookup_instance("snappy").expect("snappy in instance library");
    let cascade =
        compose_cascade(&instance, &[]).expect("snappy cascade composes");
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
        stl_objects(),
        std::env::temp_dir().display().to_string(),
        vec![],
    );

    let err = run_blocking(input, &registry, sink).expect_err("ok");
    use n3o_slic3r_lib::core::slice::SliceStartError;
    assert!(
        matches!(err, SliceStartError::NoPlatesRequested),
        "got {err:?}"
    );
}

/// The orchestrator takes a caller-resolved `PrinterInstance`, so an unknown
/// `printer_instance_id` can no longer reach it — it's rejected earlier, at
/// resolution: `lookup_instance` returns `None`, which the command /
/// `build_slice_input` turn into an `UnboundPrinter` error before a job is ever
/// started. (That boundary rejection is covered by the `build_slice_input`
/// unbound-printer test.)
#[test]
fn unknown_printer_instance_is_rejected_at_resolution() {
    use n3o_slic3r_lib::core::printer::lookup_instance;
    ensure_ffi_init(); // seeds the bundled fixture registry
    assert!(
        lookup_instance("ghost-printer").is_none(),
        "an unknown instance id must not resolve, so it can't reach the orchestrator",
    );
}

/// The U1 material-routing migration's contract, against the real engine:
/// a per-material layout with a NON-identity binding (M1 → toolhead 2,
/// M2 → toolhead 0) must slice to **logical** tool numbers (`T0`/`T1`),
/// with `filament_map` carrying the physical toolheads for planning only.
/// The legacy slot-fanned path would have rewritten the objects to
/// filaments 3/1 and emitted `T2` — so `T2` absent + `T1` present is what
/// distinguishes the two. (MAP_TABLE, sent at print start, does the
/// physical routing the engine no longer bakes in.)
#[test]
fn u1_per_material_keeps_tool_numbers_logical() {
    use n3o_slic3r_lib::core::printer::SlotRef;
    ensure_ffi_init();

    // Two materials, laid out apart so both slice; extruder = material
    // index (identity, as build_slice_input emits for a firmware-routed
    // U1). M2's object is translated +40mm in X (column-major elem 12).
    let base = stl_objects().pop().unwrap();
    let mut b_transform = IDENTITY16;
    b_transform[12] = 40.0;
    let objects = vec![
        SliceObject {
            extruder: 1,
            ..base.clone()
        },
        SliceObject {
            name: "box-m2".into(),
            extruder: 2,
            transform: b_transform,
            ..base
        },
    ];

    let td = std::env::temp_dir().join(format!("n3o-u1-logical-{}", std::process::id()));
    let mut input = snappy_input(objects, td.display().to_string(), vec![1]);
    // Per-material path: M1 → toolhead 2, M2 → toolhead 0; one filament
    // per material.
    input.material_layout = vec![
        Some(SlotRef { extruder: 2, slot: 0 }),
        Some(SlotRef { extruder: 0, slot: 0 }),
    ];
    input.context.filaments = vec![canonical_filament(), canonical_filament()];

    let gcode = slice_to_gcode("u1-logical", input);

    // filament_map = bound toolheads + 1 (planning only): [2,0] → "3,1".
    assert_eq!(config_value(&gcode, "filament_map"), "3");
    assert!(
        gcode.contains("; filament_map = 3,1"),
        "filament_map should carry both bound toolheads (3,1)",
    );
    // The emitted toolchanges are logical filament indices: T1 (the
    // second filament) is present; the physical toolhead 2 (`T2`) — which
    // the legacy remap would have emitted — never is.
    let toolchange = |t: &str| gcode.lines().any(|l| l.trim() == t);
    assert!(toolchange("T1"), "logical toolchange T1 expected");
    assert!(
        !toolchange("T2") && !toolchange("T3"),
        "no physical-toolhead tool select — numbers stay logical",
    );
}

/// Per-filament settings must survive `Print::apply`'s variant rewrite on a
/// multi-extruder printer. For every key in libslic3r's
/// `filament_options_with_variant` (nozzle temperatures, flow ratio, max
/// volumetric speed, the filament retraction overrides) the engine replaces
/// filament *f*'s value with `value[get_index_for_extruder(f,
/// "filament_self_index", …, "filament_extruder_variant")]`. That lookup
/// matches on the variant string, which is identical across a uniform
/// toolchanger, so without `filament_self_index` to disambiguate it resolves
/// every filament to index 0 and the whole vector collapses to filament 0's
/// value — two filaments at different temperatures both printed at the first
/// one's.
///
/// Composer-level assertions can't catch this: the composed cascade was
/// always right — the loss happens inside the engine. Hence the real slice.
#[test]
fn u1_per_filament_settings_survive_the_engine_variant_rewrite() {
    use n3o_slic3r_lib::core::printer::{lookup_instance, SlotRef};
    ensure_ffi_init();

    let base = stl_objects().pop().unwrap();
    let mut b_transform = IDENTITY16;
    b_transform[12] = 40.0;
    let objects = vec![
        SliceObject { extruder: 1, ..base.clone() },
        SliceObject {
            name: "box-m2".into(),
            extruder: 2,
            transform: b_transform,
            ..base
        },
    ];

    // Two toolheads carrying filaments whose profiles disagree on
    // temperature (PLA 220 / PETG 255 as bundled today).
    let mut instance = lookup_instance("snappy").expect("snappy fixture");
    instance.extruders[0].slots[0].filament_identity = Some("generic-petg".into());
    instance.extruders[1].slots[0].filament_identity = Some("generic-pla".into());

    let td = std::env::temp_dir().join(format!("n3o-u1-variant-{}", std::process::id()));
    let mut input = snappy_input(objects, td.display().to_string(), vec![1]);
    // Filament 0 → toolhead 1 (PLA), filament 1 → toolhead 0 (PETG).
    input.material_layout = vec![
        Some(SlotRef { extruder: 1, slot: 0 }),
        Some(SlotRef { extruder: 0, slot: 0 }),
    ];
    input.context.filaments = vec![canonical_filament(), canonical_filament()];

    let gcode = slice_to_gcode_with("u1-variant", input, &instance);

    // The id vector that makes the engine's per-filament variant lookup
    // resolve to each filament's own index.
    for key in ["nozzle_temperature", "nozzle_temperature_initial_layer"] {
        let temps: Vec<&str> = config_vector(&gcode, key).split(',').collect();
        assert_eq!(temps.len(), 2, "{key}: one entry per filament, got {temps:?}");
        assert_ne!(
            temps[0], temps[1],
            "{key}: both filaments got filament 0's value ({temps:?}) — the engine's \
             variant rewrite collapsed the vector",
        );
    }

    // …and the collapse would also have been invisible in the body, so pin
    // the actual heat command for the second filament.
    let second = config_vector(&gcode, "nozzle_temperature")
        .split(',')
        .nth(1)
        .unwrap()
        .to_owned();
    assert!(
        gcode.lines().any(|l| l.starts_with(&format!("M109 S{second} T1"))),
        "expected the second filament's toolhead to heat to its own temperature ({second})",
    );

    // The id vector that makes the engine's lookup resolve each filament to
    // its own index — asserted last so a regression reports the symptom
    // above, not just the missing mechanism.
    assert_eq!(config_vector(&gcode, "filament_self_index"), "1,2");
}

/// Manual PA calibration slices a swept-K test print. Snappy (U1, Klipper
/// flavor) emits `SET_PRESSURE_ADVANCE ADVANCE=<k>` per line; the sweep must
/// span the requested range. Proves the FFI calib-arming path end-to-end.
#[test]
fn pa_calibration_slices_klipper_pa_sweep() {
    use n3o_slic3r_lib::core::printer::{lookup_instance, SlotRef};
    use n3o_slic3r_lib::core::slice::pa_calibration::slice_pa_calibration;
    ensure_ffi_init();
    let instance = lookup_instance("snappy").expect("snappy fixture");
    let out = std::env::temp_dir().join("n3o-test-pa-cali-snappy.gcode");
    slice_pa_calibration(&instance, SlotRef { extruder: 0, slot: 0 }, 0.0, 0.08, 0.02, &out)
        .expect("PA calibration slice succeeds");
    let gcode = std::fs::read_to_string(&out).expect("read calib gcode");

    let mut ks: Vec<f64> = gcode
        .lines()
        .filter_map(|l| l.trim().strip_prefix("SET_PRESSURE_ADVANCE ADVANCE="))
        .filter_map(|v| v.split(';').next())
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect();
    assert!(
        ks.len() >= 3,
        "expected several SET_PRESSURE_ADVANCE lines, got {}: {ks:?}",
        ks.len(),
    );
    ks.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // The sweep brackets the requested range (libslic3r resets PA to 0 between
    // lines, so 0 is present; the top line is at/below `end`).
    assert!(*ks.last().unwrap() > 0.0, "sweep has a nonzero top K: {ks:?}");
    assert!(
        *ks.last().unwrap() <= 0.08 + 1e-9,
        "top K within requested end: {ks:?}",
    );
    let _ = std::fs::remove_file(&out);
}

/// Bambi (Bambu flavor) emits the sweep as `M900 K<k>` instead — same
/// generator, flavor-dispatched command.
#[test]
fn pa_calibration_slices_bambu_pa_sweep() {
    use n3o_slic3r_lib::core::printer::{lookup_instance, SlotRef};
    use n3o_slic3r_lib::core::slice::pa_calibration::slice_pa_calibration;
    ensure_ffi_init();
    let instance = lookup_instance("bambi").expect("bambi fixture");
    let out = std::env::temp_dir().join("n3o-test-pa-cali-bambi.gcode");
    slice_pa_calibration(&instance, SlotRef { extruder: 0, slot: 0 }, 0.0, 0.05, 0.01, &out)
        .expect("PA calibration slice succeeds");
    let gcode = std::fs::read_to_string(&out).expect("read calib gcode");
    let k_lines = gcode
        .lines()
        .filter(|l| l.trim_start().starts_with("M900 K"))
        .count();
    assert!(k_lines >= 3, "expected several M900 K lines, got {k_lines}");
    let _ = std::fs::remove_file(&out);
}
