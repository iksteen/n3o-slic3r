//! Phase 2 Rust-side smoke (PR-2-12).
//!
//! Drives the loader → scene → arrange path end-to-end without the
//! UI — the steps that don't require a human-driven GUI:
//!
//! - Active printer set, bed visualization derived.
//! - Procedural primitive added, ends up on the plate.
//! - 4-color benchy .3mf loads with the expected per-part extruder
//!   pattern (PR-0.5-3 finding).
//! - Stormtrooper Helmet 47 MB .3mf loads in under the documented
//!   budget *when staged*; skipped with a clear message when the
//!   fixture isn't present.
//!
//! The renderer-side smoke steps (viewport renders bed, drag gizmo,
//! click library catalog cube, Frame All) need a human at the
//! keyboard with `npm run tauri dev` — they're listed in the smoke
//! doc for the reviewer to walk through.

use std::path::PathBuf;
use std::time::Instant;

use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::project::{Project, Session};
use n3o_slic3r_lib::core::scene::events::SceneEvent;
use n3o_slic3r_lib::core::scene::primitives::{PrimitiveKind, PrimitiveParams};
use n3o_slic3r_lib::core::scene::state::NewSceneObject;
use n3o_slic3r_lib::core::threemf::load_3mf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn a1_mini() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu Lab A1 mini".into(),
        supported_build_plates: vec!["Textured PEI".into()],
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

#[test]
fn step_1_set_active_printer_emits_bed_changed() {
    let mut state = Session::new(Project::default());
    let events = state.set_active_printer(Some(&a1_mini()));
    let bed_set = events
        .iter()
        .any(|e| matches!(e, SceneEvent::BedChanged { bed: Some(_), .. }));
    assert!(
        bed_set,
        "set_active_printer should emit a populated BedChanged"
    );
    let plate_id = state.project.active_plate().id;
    assert!(
        state
            .plate_runtime(plate_id)
            .and_then(|r| r.bed.clone())
            .is_some(),
        "runtime has the bed cached"
    );
}

#[test]
fn step_2_library_primitive_lands_on_plate() {
    let mut state = Session::new(Project::default());
    state.set_active_printer(Some(&a1_mini()));
    let (_mesh_id, obj_id, events) = state.add_from_primitive(
        PrimitiveKind::Cube,
        PrimitiveParams::defaults_for(PrimitiveKind::Cube),
        None,
    );
    let obj = state
        .project
        .active_plate()
        .scene
        .objects
        .get(&obj_id)
        .expect("registered");
    let mesh = state.project.meshes.get(&obj.mesh).expect("registered");
    // Cube primitive bbox is [-10, 10] cubed by default; after the
    // add_from_primitive auto-lift, the world-space min Z should be
    // 0 (rests on the plate).
    let mut min_z = f32::INFINITY;
    for &x in &[
        mesh.bounding_box.min[0] as f32,
        mesh.bounding_box.max[0] as f32,
    ] {
        for &y in &[
            mesh.bounding_box.min[1] as f32,
            mesh.bounding_box.max[1] as f32,
        ] {
            for &z in &[
                mesh.bounding_box.min[2] as f32,
                mesh.bounding_box.max[2] as f32,
            ] {
                let p = obj.transform.apply_point(glam::Vec3::new(x, y, z));
                if p.z < min_z {
                    min_z = p.z;
                }
            }
        }
    }
    assert!(
        min_z.abs() < 1e-4,
        "cube should rest on plate (min_z={min_z})"
    );
    // No OOB warning expected.
    let oob = events
        .iter()
        .any(|e| matches!(e, SceneEvent::ObjectOutOfBounds { .. }));
    assert!(!oob, "primitive should land cleanly: {events:?}");
}

#[test]
fn step_3_fourcolor_3mf_loads_with_extruder_pattern() {
    let fixture = workspace_root().join("examples/spike3/fourcolor.3mf");
    if !fixture.exists() {
        eprintln!("skipping: fourcolor.3mf not present at {fixture:?}");
        return;
    }
    let project = load_3mf(&fixture).expect("load");
    assert_eq!(project.objects.len(), 8);
    let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
    assert_eq!(
        extruders,
        vec![
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
        ]
    );
}

#[test]
fn step_4_stormtrooper_loads_under_budget_when_present() {
    // Per the smoke procedure: "47 MB Stormtrooper Helmet fixture
    // staged at examples/perf-fixture/stormtrooper-helmet.3mf — mesh
    // loads in < 3 s." User must stage the file separately because of
    // its CC-BY-NC license; the test gracefully skips if missing.
    let fixture = workspace_root().join("examples/perf-fixture/stormtrooper-helmet.3mf");
    if !fixture.exists() {
        eprintln!("skipping: stormtrooper helmet fixture not staged at {fixture:?}");
        return;
    }
    let start = Instant::now();
    let project = load_3mf(&fixture).expect("load");
    let elapsed = start.elapsed();
    println!(
        "stormtrooper load: {} objects, {} meshes, {:?}",
        project.objects.len(),
        project.meshes.len(),
        elapsed,
    );
    assert!(
        elapsed.as_secs_f64() < 3.0,
        "stormtrooper helmet load exceeded 3 s budget: {elapsed:?}",
    );
    assert!(!project.objects.is_empty(), "expected at least one object");
}

#[test]
fn step_5_scene_snapshot_round_trips_after_full_setup() {
    let mut state = Session::new(Project::default());
    state.set_active_printer(Some(&a1_mini()));
    let _ = state.add_from_primitive(
        PrimitiveKind::Cube,
        PrimitiveParams::defaults_for(PrimitiveKind::Cube),
        None,
    );
    // Snapshot via the same shape `scene_snapshot` would assemble.
    let meshes: Vec<_> = state.project.meshes.values().map(|m| m.header()).collect();
    let objects: Vec<_> = state
        .project
        .active_plate()
        .scene
        .objects
        .values()
        .cloned()
        .collect();
    assert!(!meshes.is_empty());
    assert!(!objects.is_empty());
    // Each header should match an object's mesh ref.
    for obj in &objects {
        assert!(meshes.iter().any(|h| h.id == obj.mesh));
    }
}

#[test]
fn step_6_auto_arrange_then_oob_clear_under_active_printer() {
    let mut session = Session::new(Project::default());
    session.set_active_printer(Some(&a1_mini()));
    for _ in 0..6 {
        let _ = session.add_from_primitive(
            PrimitiveKind::Cube,
            PrimitiveParams {
                width: 20.0,
                depth: 20.0,
                height: 20.0,
                radius: 0.0,
                radial_segments: 0,
            },
            None,
        );
    }
    let bed = session.active_plate_runtime().bed.clone().unwrap();
    let plan = n3o_slic3r_lib::core::scene::arrange::plan_arrangement(
        &session.project,
        &bed,
        n3o_slic3r_lib::core::scene::arrange::ArrangeOptions::default(),
        session.active_plate_instance().as_ref(),
    );
    assert!(plan.un_placed.is_empty(), "6 small cubes should fit");
    let (events, _) = n3o_slic3r_lib::core::scene::arrange::apply_arrangement(&mut session, plan);
    let oob = events
        .iter()
        .filter(|e| matches!(e, SceneEvent::ObjectOutOfBounds { .. }))
        .count();
    assert_eq!(oob, 0, "arrange should leave nothing OOB");
}

#[test]
fn step_7_selection_and_delete_round_trip() {
    use n3o_slic3r_lib::core::scene::events::SelectMode;
    let mut state = Session::new(Project::default());
    state.set_active_printer(Some(&a1_mini()));
    let (_, a, _) = state.add_from_primitive(
        PrimitiveKind::Cube,
        PrimitiveParams::defaults_for(PrimitiveKind::Cube),
        None,
    );
    let (_, b, _) = state.add_from_primitive(
        PrimitiveKind::Cube,
        PrimitiveParams::defaults_for(PrimitiveKind::Cube),
        None,
    );
    state.select(&[a, b], SelectMode::Replace);
    let events = state.delete_objects(&[a, b]);
    let removed = events
        .iter()
        .filter(|e| matches!(e, SceneEvent::ObjectRemoved { .. }))
        .count();
    assert_eq!(removed, 2);
    assert_eq!(
        state.project.active_plate().scene.objects.len(),
        0,
        "both objects gone"
    );
}

#[test]
fn step_8_scene_clone_for_reconnect_drops_buffers_from_snapshot() {
    // The reconnect path serializes the scene state to JSON; mesh
    // vertex/normal/index buffers are `#[serde(skip)]` so they don't
    // bloat the snapshot. The renderer fetches them per-mesh via
    // `scene_mesh_buffers`. Verify a JSON round-trip strips them.
    let mut state = Session::new(Project::default());
    state.set_active_printer(Some(&a1_mini()));
    let (mesh_id, _, _) = state.add_from_primitive(
        PrimitiveKind::Cube,
        PrimitiveParams::defaults_for(PrimitiveKind::Cube),
        None,
    );
    let original = state.project.meshes.get(&mesh_id).unwrap();
    assert!(!original.vertices.is_empty(), "live mesh has vertex data");

    let json = serde_json::to_string(&state.project).expect("ser");
    let reloaded: Project = serde_json::from_str(&json).expect("de");
    let restored = reloaded.meshes.get(&mesh_id).unwrap();
    assert!(
        restored.vertices.is_empty(),
        "snapshot json must not carry vertex buffers"
    );
    assert_eq!(
        restored.bounding_box.min, original.bounding_box.min,
        "snapshot json preserves bbox"
    );
}

/// Silence the unused-import lint when only the parameterized
/// fixture path is referenced — `NewSceneObject` is part of the
/// public state crate surface this test exercises.
#[allow(dead_code)]
fn _retained_import_only() -> NewSceneObject {
    NewSceneObject::at_origin(n3o_slic3r_lib::core::scene::state::MeshId(1), "noop")
}
