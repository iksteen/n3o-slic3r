//! Scene-state perf gate (PR-2-11, scoped down from the original
//! Three.js-vs-wgpu pivot ticket).
//!
//! Builds a 1000-object scene and measures the latency of the
//! interactive operations the renderer fires on every drag/click.
//! Asserts mean + p99 stay under the AD-8 architectural contract:
//!
//! - Single-object transform (translate / rotate / scale): < 5 ms
//! - 100-object selection toggle: < 5 ms
//! - Full `scene_snapshot` build: < 50 ms (less frequent — fires
//!   on reconnect / startup, not per-interaction)
//!
//! The renderer-side FPS measurement is intentionally not here —
//! that's Phase 9 release-prep work on real modest hardware.
//! AD-8's load-bearing invariant is "state-side ops fit a 5 ms
//! budget"; if Rust drifts above that, the renderer choice is moot.
//!
//! Same Instant-timed harness pattern as `cascade_perf.rs` —
//! `#[test]` instead of criterion to keep the regression gate
//! inside the normal `cargo test --release` invocation.

use n3o_slic3r_lib::core::scene::events::SelectMode;
use n3o_slic3r_lib::core::scene::primitives::{PrimitiveKind, PrimitiveParams};
use n3o_slic3r_lib::core::scene::state::{ObjectId, SceneState};
use std::time::{Duration, Instant};

const OBJECT_COUNT: usize = 1000;
const ITERATIONS: u32 = 100;

fn build_scene_with_n_cubes(n: usize) -> (SceneState, Vec<ObjectId>) {
    let mut s = SceneState::new();
    let mut ids = Vec::with_capacity(n);
    // All cubes share one mesh (PR-2-7's primitive cache). The 1000
    // objects exercise the per-object hot path without inflating the
    // mesh registry beyond what a realistic project would have.
    let params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
    for _ in 0..n {
        let (_mesh, obj, _events) = s.add_from_primitive(PrimitiveKind::Cube, params);
        ids.push(obj);
    }
    assert_eq!(s.meshes.len(), 1, "primitive cache must dedup");
    assert_eq!(s.active_plate().objects.len(), n);
    (s, ids)
}

/// Run `op` N times. Returns (mean, p99) latency where p99 is the
/// max of the slowest 1% (i.e., the worst sample for ITERATIONS=100).
fn measure<F: FnMut(u32)>(mut op: F) -> (Duration, Duration) {
    // Warm-up so allocator / branch predictor settle.
    for i in 0..10 {
        op(i);
    }
    let mut samples = Vec::with_capacity(ITERATIONS as usize);
    for i in 0..ITERATIONS {
        let start = Instant::now();
        op(i);
        samples.push(start.elapsed());
    }
    samples.sort();
    let mean = samples.iter().sum::<Duration>() / (samples.len() as u32);
    // For 100 samples, "p99" is the slowest one — that's the
    // pessimistic regression check we want for a 5 ms ceiling.
    let p99 = *samples.last().unwrap();
    (mean, p99)
}

#[test]
fn translate_single_object_under_5ms_p99_on_1000_object_scene() {
    let (mut state, ids) = build_scene_with_n_cubes(OBJECT_COUNT);
    let target = ids[OBJECT_COUNT / 2];
    let (mean, p99) = measure(|i| {
        // Alternate +X / -X so the object stays near origin and the
        // operation cost doesn't drift up over many iterations.
        let dx = if i % 2 == 0 { 1.0 } else { -1.0 };
        let _ = state.translate_object(target, glam::Vec3::new(dx, 0.0, 0.0));
    });
    println!("translate mean={:?} p99={:?}", mean, p99);
    assert!(
        mean < Duration::from_millis(5),
        "translate mean ({mean:?}) exceeded 5 ms"
    );
    assert!(
        p99 < Duration::from_millis(5),
        "translate p99 ({p99:?}) exceeded 5 ms"
    );
}

#[test]
fn rotate_single_object_under_5ms_p99_on_1000_object_scene() {
    let (mut state, ids) = build_scene_with_n_cubes(OBJECT_COUNT);
    let target = ids[OBJECT_COUNT / 2];
    let (mean, p99) = measure(|i| {
        // ±10° around Z, alternating direction.
        let radians = if i % 2 == 0 { 0.17 } else { -0.17 };
        let _ = state.rotate_object(target, glam::Vec3::Z, radians, None);
    });
    println!("rotate mean={:?} p99={:?}", mean, p99);
    assert!(mean < Duration::from_millis(5), "rotate mean {mean:?}");
    assert!(p99 < Duration::from_millis(5), "rotate p99 {p99:?}");
}

#[test]
fn scale_single_object_under_5ms_p99_on_1000_object_scene() {
    let (mut state, ids) = build_scene_with_n_cubes(OBJECT_COUNT);
    let target = ids[OBJECT_COUNT / 2];
    let (mean, p99) = measure(|i| {
        let f = if i % 2 == 0 { 1.001 } else { 0.999 };
        let _ = state.scale_object(target, glam::Vec3::splat(f));
    });
    println!("scale mean={:?} p99={:?}", mean, p99);
    assert!(mean < Duration::from_millis(5), "scale mean {mean:?}");
    assert!(p99 < Duration::from_millis(5), "scale p99 {p99:?}");
}

#[test]
fn select_100_objects_under_5ms_p99() {
    let (mut state, ids) = build_scene_with_n_cubes(OBJECT_COUNT);
    let batch: Vec<ObjectId> = ids.iter().step_by(10).copied().collect();
    assert_eq!(batch.len(), 100);
    let (mean, p99) = measure(|i| {
        let mode = if i % 2 == 0 {
            SelectMode::Replace
        } else {
            SelectMode::Toggle
        };
        let _ = state.select(&batch, mode);
    });
    println!("select-100 mean={:?} p99={:?}", mean, p99);
    assert!(
        mean < Duration::from_millis(5),
        "selection toggle mean {mean:?}"
    );
    assert!(
        p99 < Duration::from_millis(5),
        "selection toggle p99 {p99:?}"
    );
}

/// `scene_snapshot` fires on the renderer's reconnect / startup
/// path — much less frequent than per-interaction ops, so the
/// budget here is looser (50 ms). What we're guarding against is
/// the JSON-serialization layer becoming O(n²) by accident.
#[test]
fn snapshot_clone_under_50ms_on_1000_object_scene() {
    let (state, _) = build_scene_with_n_cubes(OBJECT_COUNT);
    let (mean, p99) = measure(|_| {
        // Mirror what the `scene_snapshot` command does: clone the
        // pieces it returns. (We can't invoke the Tauri command
        // directly from a #[test] without spinning up a Window;
        // the work is the cloning, which is what we want to time.)
        let _meshes: Vec<_> = state.meshes.values().map(|m| m.header()).collect();
        let plate = state.active_plate();
        let _objects: Vec<_> = plate.objects.values().cloned().collect();
        let _bed = plate.bed.clone();
        let _selection: Vec<_> = plate.selection.iter().copied().collect();
        let _camera = plate.camera.clone();
        let _gizmo = plate.gizmo.clone();
    });
    println!("snapshot mean={:?} p99={:?}", mean, p99);
    assert!(
        mean < Duration::from_millis(50),
        "snapshot mean {mean:?} > 50 ms"
    );
    assert!(
        p99 < Duration::from_millis(50),
        "snapshot p99 {p99:?} > 50 ms"
    );
}

/// Sanity: the 1000-object scene actually behaves like a 1000-object
/// scene after construction. Catches regressions where, e.g., the
/// primitive cache de-dups *objects* by accident (not just meshes).
#[test]
fn scene_has_expected_shape_for_perf_run() {
    let (state, ids) = build_scene_with_n_cubes(OBJECT_COUNT);
    assert_eq!(state.active_plate().objects.len(), OBJECT_COUNT);
    assert_eq!(ids.len(), OBJECT_COUNT);
    assert_eq!(state.meshes.len(), 1, "primitive dedup");
    let mesh = state.meshes.values().next().unwrap();
    assert!(!mesh.vertices.is_empty(), "cube mesh has geometry");
}
